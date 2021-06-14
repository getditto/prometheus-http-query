use crate::error::{Error, ResponseError, UnknownResponseStatus, UnsupportedResponseDataType};
use crate::response::*;
use crate::util::validate_duration;
use std::collections::HashMap;

/// A helper enum that is passed to the `Client::new` function in
/// order to avoid errors on unsupported connection schemes.
pub enum Scheme {
    Http,
    Https,
}

impl Scheme {
    fn as_str(&self) -> &str {
        match self {
            Scheme::Http => "http",
            Scheme::Https => "https",
        }
    }
}

/// A client used to execute queries. It uses a `reqwest::Client` internally
/// that manages connections for us.
pub struct Client {
    pub(crate) client: reqwest::Client,
    pub(crate) base_url: String,
}

impl Default for Client {
    /// Create a Client that connects to a local Prometheus instance at port 9090.
    ///
    /// ```rust
    /// use prometheus_http_query::Client;
    ///
    /// let client: Client = Default::default();
    /// ```
    fn default() -> Self {
        Client {
            client: reqwest::Client::new(),
            base_url: String::from("http://127.0.0.1:9090/api/v1"),
        }
    }
}

impl Client {
    /// Create a Client that connects to a Prometheus instance at the
    /// given FQDN/domain and port, using either HTTP or HTTPS.
    ///
    /// Note that possible errors regarding domain name resolution or
    /// connection establishment will only be propagated from the underlying
    /// `reqwest::Client` when a query is executed.
    ///
    /// ```rust
    /// use prometheus_http_query::{Client, Scheme};
    ///
    /// let client = Client::new(Scheme::Http, "localhost", 9090);
    /// ```
    pub fn new(scheme: Scheme, host: &str, port: u16) -> Self {
        Client {
            base_url: format!("{}://{}:{}/api/v1", scheme.as_str(), host, port),
            ..Default::default()
        }
    }

    pub async fn query(
        &self,
        query: String,
        time: Option<i64>,
        timeout: Option<&str>,
    ) -> Result<Response, Error> {
        let mut url = self.base_url.clone();

        url.push_str("/query");

        let mut params = vec![("query", query.as_str())];

        let time = time.map(|t| t.to_string());

        if let Some(t) = &time {
            params.push(("time", t.as_str()));
        }

        if let Some(t) = timeout {
            validate_duration(t)?;
            params.push(("timeout", t));
        }

        let raw_response = self
            .client
            .get(&url)
            .query(params.as_slice())
            .send()
            .await
            .map_err(Error::Reqwest)?;

        // NOTE: Can be changed to .map(async |resp| resp.json ...)
        // when async closures are stable.
        let mapped_response = match raw_response.error_for_status() {
            Ok(res) => res
                .json::<HashMap<String, serde_json::Value>>()
                .await
                .map_err(Error::Reqwest)?,
            Err(err) => return Err(Error::Reqwest(err)),
        };

        parse_response(mapped_response)
    }

    pub async fn query_range(
        &self,
        query: String,
        start: i64,
        end: i64,
        step: &str,
        timeout: Option<&str>,
    ) -> Result<Response, Error> {
        let mut url = self.base_url.clone();

        url.push_str("/query");

        validate_duration(step)?;

        let start = start.to_string();
        let end = end.to_string();

        let mut params = vec![
            ("query", query.as_str()),
            ("start", start.as_str()),
            ("end", end.as_str()),
            ("step", step),
        ];

        if let Some(t) = timeout {
            validate_duration(t)?;
            params.push(("timeout", t));
        }

        let raw_response = self
            .client
            .get(&url)
            .query(params.as_slice())
            .send()
            .await
            .map_err(Error::Reqwest)?;

        // NOTE: Can be changed to .map(async |resp| resp.json ...)
        // when async closures are stable.
        let mapped_response = match raw_response.error_for_status() {
            Ok(res) => res
                .json::<HashMap<String, serde_json::Value>>()
                .await
                .map_err(Error::Reqwest)?,
            Err(err) => return Err(Error::Reqwest(err)),
        };

        parse_response(mapped_response)
    }
}

fn parse_response(response: HashMap<String, serde_json::Value>) -> Result<Response, Error> {
    let status = response["status"].as_str().unwrap();

    match status {
        "success" => {
            let data_obj = response["data"].as_object().unwrap();
            let data_type = data_obj["resultType"].as_str().unwrap();
            let data = data_obj["result"].as_array().unwrap();

            match data_type {
                "vector" => {
                    let mut result: Vec<VectorSample> = vec![];

                    for datum in data {
                        let mut labels: HashMap<String, String> = HashMap::new();

                        for metric in datum["metric"].as_object().unwrap() {
                            labels.insert(
                                metric.0.to_string(),
                                metric.1.as_str().unwrap().to_string(),
                            );
                        }

                        let raw_value = datum["value"].as_array().unwrap();

                        let value = Value {
                            timestamp: raw_value[0].as_f64().unwrap(),
                            value: raw_value[1].as_str().unwrap().to_string(),
                        };

                        result.push(VectorSample { labels, value });
                    }

                    Ok(Response::Vector(result))
                }
                "matrix" => {
                    let mut result: Vec<MatrixSample> = vec![];

                    for datum in data {
                        let mut labels: HashMap<String, String> = HashMap::new();

                        for metric in datum["metric"].as_object().unwrap() {
                            labels.insert(
                                metric.0.to_string(),
                                metric.1.as_str().unwrap().to_string(),
                            );
                        }

                        let mut values: Vec<Value> = vec![];

                        for value in datum["values"].as_array().unwrap() {
                            values.push(Value {
                                timestamp: value[0].as_f64().unwrap(),
                                value: value[1].as_str().unwrap().to_string(),
                            });
                        }

                        result.push(MatrixSample { labels, values });
                    }

                    Ok(Response::Matrix(result))
                }
                _ => {
                    return Err(Error::UnsupportedResponseDataType(
                        UnsupportedResponseDataType(data_type.to_string()),
                    ))
                }
            }
        }
        "error" => {
            return Err(Error::ResponseError(ResponseError {
                kind: response["errorType"].as_str().unwrap().to_string(),
                message: response["error"].as_str().unwrap().to_string(),
            }))
        }
        _ => {
            return Err(Error::UnknownResponseStatus(UnknownResponseStatus(
                status.to_string(),
            )))
        }
    }
}
