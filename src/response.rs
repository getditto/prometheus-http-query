//! All types that are returned when querying the Prometheus API.
use crate::util::{AlertState, RuleHealth, TargetHealth};
use enum_as_inner::EnumAsInner;
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt;
use time::{Duration, OffsetDateTime, PrimitiveDateTime};
use url::Url;

mod de {
    use serde::{
        de::{Error as SerdeError, Unexpected},
        Deserialize, Deserializer,
    };
    use std::str::FromStr;
    use time::format_description::FormatItem;
    use time::macros::format_description;
    use time::{Duration, PrimitiveDateTime};

    const BUILD_INFO_DATE_FORMAT: &[FormatItem] = format_description!(
        "[year repr:full][month repr:numerical][day]-[hour repr:24]:[minute]:[second]"
    );

    pub(super) fn deserialize_f64<'de, D>(deserializer: D) -> Result<f64, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).and_then(|s| {
            f64::from_str(&s).map_err(|_| {
                SerdeError::invalid_value(
                    Unexpected::Str(&s),
                    &"a float value inside a quoted JSON string",
                )
            })
        })
    }

    // This function is used to deserialize a specific datetime string like "20191102-16:19:59".
    pub(super) fn deserialize_build_info_date<'de, D>(
        deserializer: D,
    ) -> Result<PrimitiveDateTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).and_then(|s| {
            PrimitiveDateTime::parse(&s, &BUILD_INFO_DATE_FORMAT).map_err(|_| {
                SerdeError::invalid_value(
                    Unexpected::Str(&s),
                    &"a datetime string in format <yyyymmdd-hh:mm:ss>",
                )
            })
        })
    }

    // This function is used to deserialize Prometheus duration strings like "1d" or "5m" or
    // composits like "1d12h10m".
    // Note that this function assumes that the input string is non-empty and that the total
    // amount of milliseconds does not exceed i64::MAX. This seems to be a reasonable assumption
    // since the Prometheus server creates durations from Go's int64 on the server side and the
    // int64 depicts the total amount of nanoseconds.
    pub(super) fn deserialize_prometheus_duration<'de, D>(
        deserializer: D,
    ) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw_str = String::deserialize(deserializer)?;

        let mut total_milliseconds: i64 = 0;

        // Add each number character to a string until a unit character is encountered.
        // This string is then cleared to process the next number + unit.
        let mut raw_num = String::new();

        // Iterate the duration string, convert each unit to nanoseconds and add
        // it to the total.
        let mut duration_iter = raw_str.chars().peekable();

        while let Some(item) = duration_iter.next() {
            if ('0'..='9').contains(&item) {
                raw_num.push(item);
                continue;
            }

            let num = raw_num.parse::<i64>().map_err(SerdeError::custom)?;

            match item {
                'y' => {
                    total_milliseconds += num * 1000 * 60 * 60 * 24 * 365;
                }
                'w' => {
                    total_milliseconds += num * 1000 * 60 * 60 * 24 * 7;
                }
                'd' => {
                    total_milliseconds += num * 1000 * 60 * 60 * 24;
                }
                'h' => {
                    total_milliseconds += num * 1000 * 60 * 60;
                }
                'm' => {
                    if duration_iter.next_if_eq(&'s').is_some() {
                        total_milliseconds += num * 1000 * 60 * 60;
                    } else {
                        total_milliseconds += num * 1000 * 60;
                    }
                }
                's' => {
                    total_milliseconds += num * 1000;
                }
                _ => return Err(SerdeError::custom("invalid time duration")),
            };

            raw_num.clear();
        }

        Ok(Duration::milliseconds(total_milliseconds))
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status")]
pub(crate) enum ApiResponse<D> {
    #[serde(alias = "success")]
    Success { data: D },
    #[serde(alias = "error")]
    Error(crate::error::PrometheusError),
}

#[derive(Debug, Clone, Deserialize)]
pub struct Stats {
    timings: Timings,
    samples: Samples,
}

impl Stats {
    pub fn timings(&self) -> &Timings {
        &self.timings
    }

    pub fn samples(&self) -> &Samples {
        &self.samples
    }
}

#[derive(Debug, Copy, Clone, Deserialize)]
pub struct Timings {
    #[serde(alias = "evalTotalTime")]
    eval_total_time: f64,
    #[serde(alias = "resultSortTime")]
    result_sort_time: f64,
    #[serde(alias = "queryPreparationTime")]
    query_preparation_time: f64,
    #[serde(alias = "innerEvalTime")]
    inner_eval_time: f64,
    #[serde(alias = "execQueueTime")]
    exec_queue_time: f64,
    #[serde(alias = "execTotalTime")]
    exec_total_time: f64,
}

impl Timings {
    pub fn eval_total_time(&self) -> f64 {
        self.eval_total_time
    }

    pub fn result_sort_time(&self) -> f64 {
        self.result_sort_time
    }

    pub fn query_preparation_time(&self) -> f64 {
        self.query_preparation_time
    }

    pub fn inner_eval_time(&self) -> f64 {
        self.inner_eval_time
    }

    pub fn exec_queue_time(&self) -> f64 {
        self.exec_queue_time
    }

    pub fn exec_total_time(&self) -> f64 {
        self.exec_total_time
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Samples {
    #[serde(alias = "totalQueryableSamplesPerStep")]
    total_queryable_samples_per_step: Option<Vec<SamplesPerStep>>,
    #[serde(alias = "totalQueryableSamples")]
    total_queryable_samples: i64,
    #[serde(alias = "peakSamples")]
    peak_samples: i64,
}

impl Samples {
    pub fn total_queryable_samples_per_step(&self) -> Option<&Vec<SamplesPerStep>> {
        self.total_queryable_samples_per_step.as_ref()
    }

    pub fn total_queryable_samples(&self) -> i64 {
        self.total_queryable_samples
    }

    pub fn peak_samples(&self) -> i64 {
        self.peak_samples
    }
}

/// A hint to the total queryable samples in this step.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct SamplesPerStep {
    pub(crate) timestamp: f64,
    pub(crate) value: usize,
}

impl SamplesPerStep {
    /// Returns the timestamp at the start of this step.
    pub fn timestamp(&self) -> f64 {
        self.timestamp
    }

    /// Returns the number of samples in this step.
    pub fn value(&self) -> usize {
        self.value
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PromqlResult {
    #[serde(flatten)]
    pub(crate) data: Data,
    pub(crate) stats: Option<Stats>,
}

impl PromqlResult {
    /// Return the response [`Data`] from this query.
    pub fn data(&self) -> &Data {
        &self.data
    }

    /// Return the [`Stats`] that the Prometheus server gathered while the query was processed.
    pub fn stats(&self) -> Option<&Stats> {
        self.stats.as_ref()
    }

    /// Returns the inner types when ownership is required
    pub fn into_inner(self) -> (Data, Option<Stats>) {
        (self.data, self.stats)
    }
}

/// A wrapper for possible result types of expression queries ([`Client::query`](crate::Client::query) and [`Client::query_range`](crate::Client::query_range)).
#[derive(Clone, Debug, Deserialize, EnumAsInner)]
#[serde(tag = "resultType", content = "result")]
pub enum Data {
    #[serde(alias = "vector")]
    Vector(Vec<InstantVector>),
    #[serde(alias = "matrix")]
    Matrix(Vec<RangeVector>),
    #[serde(alias = "scalar")]
    Scalar(Sample),
}

impl Data {
    /// This is a shortcut to check if the query returned any data at all regardless of the exact type.
    pub fn is_empty(&self) -> bool {
        match self {
            Data::Vector(v) => v.is_empty(),
            Data::Matrix(v) => v.is_empty(),
            Data::Scalar(_) => false,
        }
    }
}

/// A single time series containing a single data point/sample.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct InstantVector {
    pub(crate) metric: HashMap<String, String>,
    #[serde(alias = "value")]
    pub(crate) sample: Sample,
}

impl InstantVector {
    /// Returns a reference to the set of labels (+ metric name)
    /// of this time series.
    pub fn metric(&self) -> &HashMap<String, String> {
        &self.metric
    }

    /// Returns a reference to the sample of this time series.
    pub fn sample(&self) -> &Sample {
        &self.sample
    }

    /// Returns the inner types when ownership is required
    pub fn into_inner(self) -> (HashMap<String, String>, Sample) {
        (self.metric, self.sample)
    }
}

/// A single time series containing a range of data points/samples.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct RangeVector {
    pub(crate) metric: HashMap<String, String>,
    #[serde(alias = "values")]
    pub(crate) samples: Vec<Sample>,
}

impl RangeVector {
    /// Returns a reference to the set of labels (+ metric name)
    /// of this time series.
    pub fn metric(&self) -> &HashMap<String, String> {
        &self.metric
    }

    /// Returns a reference to the set of samples of this time series.
    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    /// Returns the inner types when ownership is required
    pub fn into_inner(self) -> (HashMap<String, String>, Vec<Sample>) {
        (self.metric, self.samples)
    }
}

/// A single data point.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct Sample {
    pub(crate) timestamp: f64,
    #[serde(deserialize_with = "de::deserialize_f64")]
    pub(crate) value: f64,
}

impl Sample {
    /// Returns the timestamp contained in this sample.
    pub fn timestamp(&self) -> f64 {
        self.timestamp
    }

    /// Returns the value contained in this sample.
    pub fn value(&self) -> f64 {
        self.value
    }
}

/// Collection of active and dropped targets as returned by the API.
#[derive(Clone, Debug, Deserialize)]
pub struct Targets {
    #[serde(alias = "activeTargets")]
    pub(crate) active: Vec<ActiveTarget>,
    #[serde(alias = "droppedTargets")]
    pub(crate) dropped: Vec<DroppedTarget>,
}

impl Targets {
    /// Get a list of currently active targets.
    pub fn active(&self) -> &[ActiveTarget] {
        &self.active
    }

    /// Get a list of dropped targets.
    pub fn dropped(&self) -> &[DroppedTarget] {
        &self.dropped
    }
}

/// A single active target.
#[derive(Clone, Debug, Deserialize)]
pub struct ActiveTarget {
    #[serde(alias = "discoveredLabels")]
    pub(crate) discovered_labels: HashMap<String, String>,
    pub(crate) labels: HashMap<String, String>,
    #[serde(alias = "scrapePool")]
    pub(crate) scrape_pool: String,
    #[serde(alias = "scrapeUrl")]
    pub(crate) scrape_url: Url,
    #[serde(alias = "globalUrl")]
    pub(crate) global_url: Url,
    #[serde(alias = "lastError")]
    pub(crate) last_error: String,
    #[serde(alias = "lastScrape")]
    #[serde(with = "time::serde::rfc3339")]
    pub(crate) last_scrape: OffsetDateTime,
    #[serde(alias = "lastScrapeDuration")]
    pub(crate) last_scrape_duration: f64,
    pub(crate) health: TargetHealth,
    #[serde(
        alias = "scrapeInterval",
        deserialize_with = "de::deserialize_prometheus_duration"
    )]
    pub(crate) scrape_interval: Duration,
    #[serde(
        alias = "scrapeTimeout",
        deserialize_with = "de::deserialize_prometheus_duration"
    )]
    pub(crate) scrape_timeout: Duration,
}

impl ActiveTarget {
    /// Get a set of unmodified labels as before relabelling occurred.
    pub fn discovered_labels(&self) -> &HashMap<String, String> {
        &self.discovered_labels
    }

    /// Get a set of labels after relabelling.
    pub fn labels(&self) -> &HashMap<String, String> {
        &self.labels
    }

    /// Get the scrape pool of this target.
    pub fn scrape_pool(&self) -> &str {
        &self.scrape_pool
    }

    /// Get the scrape URL of this target.
    pub fn scrape_url(&self) -> &Url {
        &self.scrape_url
    }

    /// Get the global URL of this target.
    pub fn global_url(&self) -> &Url {
        &self.global_url
    }

    /// Get the last error reported for this target.
    pub fn last_error(&self) -> &str {
        &self.last_error
    }

    /// Get the time when the last scrape occurred.
    pub fn last_scrape(&self) -> &OffsetDateTime {
        &self.last_scrape
    }

    /// Get the duration that the last scrape ran for in seconds.
    pub fn last_scrape_duration(&self) -> f64 {
        self.last_scrape_duration
    }

    /// Get the health status of this target.
    pub fn health(&self) -> TargetHealth {
        self.health
    }

    /// Get the scrape interval of this target.
    pub fn scrape_interval(&self) -> &Duration {
        &self.scrape_interval
    }

    /// Get the scrape timeout of this target.
    pub fn scrape_timeout(&self) -> &Duration {
        &self.scrape_timeout
    }
}

/// A single dropped target.
#[derive(Clone, Debug, Deserialize)]
pub struct DroppedTarget {
    #[serde(alias = "discoveredLabels")]
    pub(crate) discovered_labels: HashMap<String, String>,
}

impl DroppedTarget {
    /// Get a set of unmodified labels as before relabelling occurred.
    pub fn discovered_labels(&self) -> &HashMap<String, String> {
        &self.discovered_labels
    }
}

/// This is a wrapper around a collection of [`RuleGroup`]s as it is
/// returned by the API.
#[derive(Debug, Deserialize)]
pub(crate) struct RuleGroups {
    pub groups: Vec<RuleGroup>,
}

/// A group of rules.
#[derive(Clone, Debug, Deserialize)]
pub struct RuleGroup {
    pub(crate) rules: Vec<Rule>,
    pub(crate) file: String,
    pub(crate) interval: f64,
    pub(crate) name: String,
    #[serde(alias = "evaluationTime")]
    pub(crate) evaluation_time: f64,
    #[serde(alias = "lastEvaluation", with = "time::serde::rfc3339")]
    pub(crate) last_evaluation: OffsetDateTime,
    pub(crate) limit: usize,
}

impl RuleGroup {
    /// Get a reference to all rules associated with this group.
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Get the path to the file where this group is defined in.
    pub fn file(&self) -> &str {
        &self.file
    }

    /// Get the interval that defines how often rules are evaluated.
    pub fn interval(&self) -> f64 {
        self.interval
    }

    /// Get the name of this rule group.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the time when the group of rules was last evaluated.
    pub fn last_evaluation(&self) -> &OffsetDateTime {
        &self.last_evaluation
    }

    /// Get duration in seconds that Prometheus took to evaluate this group of
    /// rules.
    pub fn evaluation_time(&self) -> f64 {
        self.evaluation_time
    }

    /// Return the limit of alerts (alerting rule) or series (recording rule)
    /// that the rules in this group may produce. Zero means not limit.
    pub fn limit(&self) -> usize {
        self.limit
    }
}

/// A wrapper for different types of rules that the HTTP API may return.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type")]
pub enum Rule {
    #[serde(alias = "recording")]
    Recording(RecordingRule),
    #[serde(alias = "alerting")]
    Alerting(AlertingRule),
}

impl Rule {
    pub fn as_recording(&self) -> Option<&RecordingRule> {
        match self {
            Self::Recording(rule) => Some(&rule),
            _ => None,
        }
    }

    pub fn as_alerting(&self) -> Option<&AlertingRule> {
        match self {
            Self::Alerting(rule) => Some(&rule),
            _ => None,
        }
    }
}

/// An alerting rule.
#[derive(Clone, Debug, Deserialize)]
pub struct AlertingRule {
    pub(crate) alerts: Vec<Alert>,
    pub(crate) annotations: HashMap<String, String>,
    pub(crate) duration: f64,
    pub(crate) health: RuleHealth,
    pub(crate) labels: HashMap<String, String>,
    pub(crate) name: String,
    pub(crate) query: String,
    #[serde(alias = "evaluationTime")]
    pub(crate) evaluation_time: f64,
    #[serde(alias = "lastEvaluation", with = "time::serde::rfc3339")]
    pub(crate) last_evaluation: OffsetDateTime,
    #[serde(alias = "keepFiringFor")]
    pub(crate) keep_firing_for: f64,
}

impl AlertingRule {
    /// Get a list of active alerts fired due to this alerting rule.
    pub fn alerts(&self) -> &[Alert] {
        &self.alerts
    }

    /// Get a set of annotations set for this rule.
    pub fn annotations(&self) -> &HashMap<String, String> {
        &self.annotations
    }

    /// Get the duration that Prometheus waits for before firing for this rule.
    pub fn duration(&self) -> f64 {
        self.duration
    }

    /// Get the health state of this rule.
    pub fn health(&self) -> RuleHealth {
        self.health
    }

    /// Get a set of labels defined for this rule.
    pub fn labels(&self) -> &HashMap<String, String> {
        &self.labels
    }

    /// Get the name of this rule.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the PromQL expression that is evaluated as part of this rule.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Get the time when the rule was last evaluated.
    pub fn last_evaluation(&self) -> &OffsetDateTime {
        &self.last_evaluation
    }

    /// Get duration in seconds that Prometheus took to evaluate this rule.
    pub fn evaluation_time(&self) -> f64 {
        self.evaluation_time
    }

    /// Get the duration that Prometheus waits before clearing an alert that
    /// has previously been firing.
    pub fn keep_firing_for(&self) -> f64 {
        self.keep_firing_for
    }
}

/// A recording rule.
#[derive(Clone, Debug, Deserialize)]
pub struct RecordingRule {
    pub(crate) health: RuleHealth,
    pub(crate) name: String,
    pub(crate) query: String,
    pub(crate) labels: Option<HashMap<String, String>>,
    #[serde(alias = "evaluationTime")]
    pub(crate) evaluation_time: f64,
    #[serde(alias = "lastEvaluation", with = "time::serde::rfc3339")]
    pub(crate) last_evaluation: OffsetDateTime,
}

impl RecordingRule {
    /// Get the health state of this rule.
    pub fn health(&self) -> RuleHealth {
        self.health
    }

    /// Get the name of this rule.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the PromQL expression that is evaluated as part of this rule.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Get a set of labels defined for this rule.
    pub fn labels(&self) -> &Option<HashMap<String, String>> {
        &self.labels
    }

    /// Get the time when the rule was last evaluated.
    pub fn last_evaluation(&self) -> &OffsetDateTime {
        &self.last_evaluation
    }

    /// Get duration in seconds that Prometheus took to evaluate this rule.
    pub fn evaluation_time(&self) -> f64 {
        self.evaluation_time
    }
}

/// A wrapper around a collection of [`Alert`]s as it is returned by
/// the API.
#[derive(Debug, Deserialize)]
pub(crate) struct Alerts {
    pub alerts: Vec<Alert>,
}

/// A single alert.
#[derive(Clone, Debug, Deserialize)]
pub struct Alert {
    #[serde(alias = "activeAt", with = "time::serde::rfc3339")]
    pub(crate) active_at: OffsetDateTime,
    pub(crate) annotations: HashMap<String, String>,
    pub(crate) labels: HashMap<String, String>,
    pub(crate) state: AlertState,
    #[serde(deserialize_with = "de::deserialize_f64")]
    pub(crate) value: f64,
}

impl Alert {
    /// Get the time when this alert started firing.
    pub fn active_at(&self) -> &OffsetDateTime {
        &self.active_at
    }

    /// Get a set of annotations associated with this alert.
    pub fn annotations(&self) -> &HashMap<String, String> {
        &self.annotations
    }

    /// Get a set of labels associated with this alert.
    pub fn labels(&self) -> &HashMap<String, String> {
        &self.labels
    }

    /// Get the state of this alert.
    pub fn state(&self) -> AlertState {
        self.state
    }

    /// Get the value as evaluated by the PromQL expression that caused the alert to fire.
    pub fn value(&self) -> f64 {
        self.value
    }
}

/// Collection of active and dropped alertmanagers as returned by the API.
#[derive(Clone, Debug, Deserialize)]
pub struct Alertmanagers {
    #[serde(alias = "activeAlertmanagers")]
    pub(crate) active: Vec<Alertmanager>,
    #[serde(alias = "droppedAlertmanagers")]
    pub(crate) dropped: Vec<Alertmanager>,
}

impl Alertmanagers {
    /// Get a list of currently active alertmanagers.
    pub fn active(&self) -> &[Alertmanager] {
        &self.active
    }

    /// Get a list of dropped alertmanagers.
    pub fn dropped(&self) -> &[Alertmanager] {
        &self.dropped
    }
}

/// A single alertmanager.
#[derive(Clone, Debug, Deserialize)]
pub struct Alertmanager {
    url: Url,
}

impl Alertmanager {
    /// Get the URL of this Alertmanager.
    pub fn url(&self) -> &Url {
        &self.url
    }
}

/// Possible metric types that the HTTP API may return.
#[derive(Debug, Copy, Clone, Deserialize, Eq, PartialEq)]
pub enum MetricType {
    #[serde(alias = "counter")]
    Counter,
    #[serde(alias = "gauge")]
    Gauge,
    #[serde(alias = "histogram")]
    Histogram,
    #[serde(alias = "gaugehistogram")]
    GaugeHistogram,
    #[serde(alias = "summary")]
    Summary,
    #[serde(alias = "info")]
    Info,
    #[serde(alias = "stateset")]
    Stateset,
    #[serde(alias = "unknown")]
    Unknown,
}

impl MetricType {
    pub fn is_counter(&self) -> bool {
        *self == Self::Counter
    }

    pub fn is_gauge(&self) -> bool {
        *self == Self::Gauge
    }

    pub fn is_histogram(&self) -> bool {
        *self == Self::Histogram
    }

    pub fn is_gauge_histogram(&self) -> bool {
        *self == Self::GaugeHistogram
    }

    pub fn is_summary(&self) -> bool {
        *self == Self::Summary
    }

    pub fn is_info(&self) -> bool {
        *self == Self::Info
    }

    pub fn is_stateset(&self) -> bool {
        *self == Self::Stateset
    }

    pub fn is_unknown(&self) -> bool {
        *self == Self::Unknown
    }
}

impl fmt::Display for MetricType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetricType::Counter => write!(f, "counter"),
            MetricType::Gauge => write!(f, "gauge"),
            MetricType::Histogram => write!(f, "histogram"),
            MetricType::GaugeHistogram => write!(f, "gaugehistogram"),
            MetricType::Summary => write!(f, "summary"),
            MetricType::Info => write!(f, "info"),
            MetricType::Stateset => write!(f, "stateset"),
            MetricType::Unknown => write!(f, "unknown"),
        }
    }
}

/// A target metadata object.
#[derive(Clone, Debug, Deserialize)]
pub struct TargetMetadata {
    pub(crate) target: HashMap<String, String>,
    #[serde(alias = "type")]
    pub(crate) metric_type: MetricType,
    pub(crate) metric: Option<String>,
    pub(crate) help: String,
    pub(crate) unit: String,
}

impl TargetMetadata {
    /// Get target labels.
    pub fn target(&self) -> &HashMap<String, String> {
        &self.target
    }

    /// Get the metric type.
    pub fn metric_type(&self) -> MetricType {
        self.metric_type
    }

    /// Get the metric name.
    pub fn metric(&self) -> Option<&str> {
        self.metric.as_deref()
    }

    /// Get the metric help.
    pub fn help(&self) -> &str {
        &self.help
    }

    /// Get the metric unit.
    pub fn unit(&self) -> &str {
        &self.unit
    }
}

/// A metric metadata object.
#[derive(Clone, Debug, Deserialize)]
pub struct MetricMetadata {
    #[serde(alias = "type")]
    pub(crate) metric_type: MetricType,
    pub(crate) help: String,
    pub(crate) unit: String,
}

impl MetricMetadata {
    /// Get the metric type.
    pub fn metric_type(&self) -> MetricType {
        self.metric_type
    }

    /// Get the metric help.
    pub fn help(&self) -> &str {
        &self.help
    }

    /// Get the metric unit.
    pub fn unit(&self) -> &str {
        &self.unit
    }
}

/// An object containing Prometheus server build information.
#[derive(Clone, Debug, Deserialize)]
pub struct BuildInformation {
    pub(crate) version: String,
    pub(crate) revision: String,
    pub(crate) branch: String,
    #[serde(alias = "buildUser")]
    pub(crate) build_user: String,
    #[serde(alias = "buildDate")]
    #[serde(deserialize_with = "de::deserialize_build_info_date")]
    pub(crate) build_date: PrimitiveDateTime,
    #[serde(alias = "goVersion")]
    pub(crate) go_version: String,
}

impl BuildInformation {
    /// Get the server version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Get the git revision from which the server was built.
    pub fn revision(&self) -> &str {
        &self.revision
    }

    /// Get the git branch from which the server was built.
    pub fn branch(&self) -> &str {
        &self.branch
    }

    /// Get the user who built the server.
    pub fn build_user(&self) -> &str {
        &self.build_user
    }

    /// Get the date at which the server was built.
    pub fn build_date(&self) -> &PrimitiveDateTime {
        &self.build_date
    }

    /// Get the Go version that was used to build the server.
    pub fn go_version(&self) -> &str {
        &self.go_version
    }
}

/// An object containing Prometheus server build information.
#[derive(Clone, Debug, Deserialize)]
pub struct RuntimeInformation {
    #[serde(alias = "startTime", with = "time::serde::rfc3339")]
    pub(crate) start_time: OffsetDateTime,
    #[serde(alias = "CWD")]
    pub(crate) cwd: String,
    #[serde(alias = "reloadConfigSuccess")]
    pub(crate) reload_config_success: bool,
    #[serde(alias = "lastConfigTime", with = "time::serde::rfc3339")]
    pub(crate) last_config_time: OffsetDateTime,
    #[serde(alias = "corruptionCount")]
    pub(crate) corruption_count: i64,
    #[serde(alias = "goroutineCount")]
    pub(crate) goroutine_count: usize,
    #[serde(alias = "GOMAXPROCS")]
    pub(crate) go_max_procs: usize,
    #[serde(alias = "GOGC")]
    pub(crate) go_gc: String,
    #[serde(alias = "GODEBUG")]
    pub(crate) go_debug: String,
    #[serde(
        alias = "storageRetention",
        deserialize_with = "de::deserialize_prometheus_duration"
    )]
    pub(crate) storage_retention: Duration,
}

impl RuntimeInformation {
    /// Get the server start time.
    pub fn start_time(&self) -> &OffsetDateTime {
        &self.start_time
    }

    /// Get the current working directory.
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// Check if the last configuration reload was successful.
    pub fn reload_config_success(&self) -> bool {
        self.reload_config_success
    }

    /// Get the time of last configuration reload.
    pub fn last_config_time(&self) -> &OffsetDateTime {
        &self.last_config_time
    }

    pub fn corruption_count(&self) -> i64 {
        self.corruption_count
    }

    pub fn goroutine_count(&self) -> usize {
        self.goroutine_count
    }

    pub fn go_max_procs(&self) -> usize {
        self.go_max_procs
    }

    pub fn go_gc(&self) -> &str {
        &self.go_gc
    }

    pub fn go_debug(&self) -> &str {
        &self.go_debug
    }

    pub fn storage_retention(&self) -> &Duration {
        &self.storage_retention
    }
}

/// Prometheus TSDB statistics.
#[derive(Clone, Debug, Deserialize)]
pub struct TsdbStatistics {
    #[serde(alias = "headStats")]
    pub(crate) head_stats: HeadStatistics,
    #[serde(alias = "seriesCountByMetricName")]
    pub(crate) series_count_by_metric_name: Vec<TsdbItemCount>,
    #[serde(alias = "labelValueCountByLabelName")]
    pub(crate) label_value_count_by_label_name: Vec<TsdbItemCount>,
    #[serde(alias = "memoryInBytesByLabelName")]
    pub(crate) memory_in_bytes_by_label_name: Vec<TsdbItemCount>,
    #[serde(alias = "seriesCountByLabelValuePair")]
    pub(crate) series_count_by_label_value_pair: Vec<TsdbItemCount>,
}

impl TsdbStatistics {
    /// Get the head block data.
    pub fn head_stats(&self) -> HeadStatistics {
        self.head_stats
    }

    /// Get a list of metric names and their series count.
    pub fn series_count_by_metric_name(&self) -> &[TsdbItemCount] {
        &self.series_count_by_metric_name
    }

    /// Get a list of label names and their value count.
    pub fn label_value_count_by_label_name(&self) -> &[TsdbItemCount] {
        &self.label_value_count_by_label_name
    }

    /// Get a list of label names and memory used in bytes.
    pub fn memory_in_bytes_by_label_name(&self) -> &[TsdbItemCount] {
        &self.memory_in_bytes_by_label_name
    }

    /// Get a list of label name/value pairs and their series count.
    pub fn series_count_by_label_value_pair(&self) -> &[TsdbItemCount] {
        &self.series_count_by_label_value_pair
    }
}

/// Prometheus TSDB head block data.
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct HeadStatistics {
    #[serde(alias = "numSeries")]
    pub(crate) num_series: usize,
    #[serde(alias = "chunkCount")]
    pub(crate) chunk_count: usize,
    #[serde(alias = "minTime")]
    pub(crate) min_time: i64,
    #[serde(alias = "maxTime")]
    pub(crate) max_time: i64,
}

impl HeadStatistics {
    /// Get the number of series.
    pub fn num_series(&self) -> usize {
        self.num_series
    }

    /// Get the number of chunks.
    pub fn chunk_count(&self) -> usize {
        self.chunk_count
    }

    /// Get the current minimum timestamp in milliseconds.
    pub fn min_time(&self) -> i64 {
        self.min_time
    }

    /// Get the current maximum timestamp in milliseconds.
    pub fn max_time(&self) -> i64 {
        self.max_time
    }
}

/// Prometheus TSDB item counts used in different contexts (e.g. series count, label value count ...).
#[derive(Clone, Debug, Deserialize)]
pub struct TsdbItemCount {
    pub(crate) name: String,
    pub(crate) value: usize,
}

impl TsdbItemCount {
    /// Get the name of the item in question, e.g. metric name or label name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the count of the item in question, e.g. the series count of a given metric name.
    pub fn value(&self) -> usize {
        self.value
    }
}

/// WAL replay state.
#[derive(Clone, Copy, Debug, Deserialize)]
pub struct WalReplayStatistics {
    pub(crate) min: usize,
    pub(crate) max: usize,
    pub(crate) current: usize,
    pub(crate) state: Option<WalReplayState>,
}

impl WalReplayStatistics {
    pub fn min(&self) -> usize {
        self.min
    }

    pub fn max(&self) -> usize {
        self.max
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn state(&self) -> Option<WalReplayState> {
        self.state
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum WalReplayState {
    #[serde(alias = "waiting")]
    Waiting,
    #[serde(alias = "in progress")]
    InProgress,
    #[serde(alias = "done")]
    Done,
}

impl WalReplayState {
    pub fn is_waiting(&self) -> bool {
        *self == Self::Waiting
    }

    pub fn is_in_progress(&self) -> bool {
        *self == Self::InProgress
    }

    pub fn is_done(&self) -> bool {
        *self == Self::Done
    }
}

#[cfg(test)]
mod tests {
    // The examples used in these test cases are partly taken from prometheus.io.
    // Others are copied from responses of live Prometheus servers.

    use super::*;
    use std::collections::HashMap;
    use time::macros::datetime;

    #[test]
    fn test_api_error_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
{
  "status": "error",
  "data": null,
  "errorType": "bad_data",
  "error": "1:14: parse error: unexpected end of input in aggregation",
  "warnings": []
}
"#;

        let result = serde_json::from_str::<ApiResponse<PromqlResult>>(data)?;
        assert!(
            matches!(result, ApiResponse::Error(err) if err.error_type == crate::error::PrometheusErrorType::BadData)
        );

        Ok(())
    }

    #[test]
    fn test_api_success_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
{
  "status": "success",
  "data": {
    "resultType": "scalar",
    "result": [ 0, "0.0" ]
  },
  "warnings": []
}
"#;

        let result = serde_json::from_str::<ApiResponse<PromqlResult>>(data)?;
        assert!(matches!(result, ApiResponse::Success { data: _ }));

        Ok(())
    }

    #[test]
    fn test_bad_combination_in_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
{
  "status": "error",
  "data": {
    "resultType": "scalar",
    "result": [ 0, "0.0" ]
  },
  "warnings": []
}
"#;

        let result = serde_json::from_str::<ApiResponse<()>>(data);
        assert!(result.is_err());

        Ok(())
    }

    #[test]
    fn test_another_bad_combination_in_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
{
  "status": "success",
  "warnings": []
  "errorType": "bad_data",
  "error": "1:14: parse error: unexpected end of input in aggregation",
}
"#;

        let result = serde_json::from_str::<ApiResponse<()>>(data);
        assert!(result.is_err());

        Ok(())
    }

    #[test]
    fn test_query_result_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
{
  "resultType": "matrix",
  "result": [
    {
      "metric": {
        "__name__": "up",
        "instance": "localhost:9090",
        "job": "prometheus"
      },
      "values": [
        [
          1659268100,
          "1"
        ],
        [
          1659268160,
          "1"
        ],
        [
          1659268220,
          "1"
        ],
        [
          1659268280,
          "1"
        ]
      ]
    }
  ],
  "stats": {
    "timings": {
      "evalTotalTime": 0.000102139,
      "resultSortTime": 8.7e-07,
      "queryPreparationTime": 5.4169e-05,
      "innerEvalTime": 3.787e-05,
      "execQueueTime": 4.07e-05,
      "execTotalTime": 0.000151989
    },
    "samples": {
      "totalQueryableSamplesPerStep": [
        [
          1659268100,
          1
        ],
        [
          1659268160,
          1
        ],
        [
          1659268220,
          1
        ],
        [
          1659268280,
          1
        ]
      ],
      "totalQueryableSamples": 4,
      "peakSamples": 4
    }
  }
}
"#;
        let result = serde_json::from_str::<PromqlResult>(data)?;
        let data = &result.data;
        assert!(data.is_matrix());
        let matrix = data.as_matrix().unwrap();
        assert!(matrix.len() == 1);
        let range_vector = &matrix[0];
        let metric = &range_vector.metric();
        assert!(metric.len() == 3);
        assert!(metric.get("__name__").is_some_and(|v| v == "up"));
        assert!(metric
            .get("instance")
            .is_some_and(|v| v == "localhost:9090"));
        assert!(metric.get("job").is_some_and(|v| v == "prometheus"));
        let samples = range_vector.samples();
        assert!(samples.len() == 4);
        assert!(samples[0].timestamp() == 1659268100.0);
        assert!(samples[0].value() == 1.0);
        assert!(samples[1].timestamp() == 1659268160.0);
        assert!(samples[1].value() == 1.0);
        assert!(samples[2].timestamp() == 1659268220.0);
        assert!(samples[2].value() == 1.0);
        assert!(samples[3].timestamp() == 1659268280.0);
        assert!(samples[3].value() == 1.0);
        assert!(result.stats().is_some());
        let stats = result.stats().unwrap();
        let timings = stats.timings();
        assert!(timings.eval_total_time() == 0.000102139);
        assert!(timings.result_sort_time() == 8.7e-07_f64);
        assert!(timings.query_preparation_time() == 5.4169e-05_f64);
        assert!(timings.inner_eval_time() == 3.787e-05_f64);
        assert!(timings.exec_queue_time() == 4.07e-05_f64);
        assert!(timings.exec_total_time() == 0.000151989);
        let samples = stats.samples();
        assert!(samples.peak_samples() == 4);
        assert!(samples.total_queryable_samples() == 4);
        assert!(samples.total_queryable_samples_per_step().is_some());
        let per_step = samples.total_queryable_samples_per_step().unwrap();
        assert!(per_step.len() == 4);
        assert!(per_step[0].timestamp() == 1659268100.0);
        assert!(per_step[0].value() == 1);
        assert!(per_step[1].timestamp() == 1659268160.0);
        assert!(per_step[1].value() == 1);
        assert!(per_step[2].timestamp() == 1659268220.0);
        assert!(per_step[2].value() == 1);
        assert!(per_step[3].timestamp() == 1659268280.0);
        assert!(per_step[3].value() == 1);
        Ok(())
    }

    #[test]
    fn test_query_result_no_per_step_stats_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
{
  "resultType": "matrix",
  "result": [
    {
      "metric": {
        "__name__": "up",
        "instance": "localhost:9090",
        "job": "prometheus"
      },
      "values": [
        [
          1659268100,
          "1"
        ],
        [
          1659268160,
          "1"
        ],
        [
          1659268220,
          "1"
        ],
        [
          1659268280,
          "1"
        ]
      ]
    }
  ],
  "stats": {
    "timings": {
      "evalTotalTime": 0.000102139,
      "resultSortTime": 8.7e-07,
      "queryPreparationTime": 5.4169e-05,
      "innerEvalTime": 3.787e-05,
      "execQueueTime": 4.07e-05,
      "execTotalTime": 0.000151989
    },
    "samples": {
      "totalQueryableSamples": 4,
      "peakSamples": 4
    }
  }
}
"#;
        let result = serde_json::from_str::<PromqlResult>(data)?;
        assert!(result.stats().is_some());
        let stats = result.stats().unwrap();
        assert!(stats.samples().total_queryable_samples_per_step().is_none());

        Ok(())
    }

    #[test]
    fn test_query_result_no_stats_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
{
  "resultType": "matrix",
  "result": [
    {
      "metric": {
        "__name__": "up",
        "instance": "localhost:9090",
        "job": "prometheus"
      },
      "values": [
        [
          1659268100,
          "1"
        ],
        [
          1659268160,
          "1"
        ],
        [
          1659268220,
          "1"
        ],
        [
          1659268280,
          "1"
        ]
      ]
    }
  ]
}
"#;
        let result = serde_json::from_str::<PromqlResult>(data)?;
        assert!(result.stats().is_none());

        Ok(())
    }

    #[test]
    fn test_instant_vector_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
[
  {
    "metric": {
      "__name__": "up",
      "job": "prometheus",
      "instance": "localhost:9090"
    },
    "value": [
      1435781451.781,
      "1"
    ]
  },
  {
    "metric": {
      "__name__": "up",
      "job": "node",
      "instance": "localhost:9100"
    },
    "value": [
      1435781451.781,
      "0"
    ]
  }
]
"#;
        serde_json::from_str::<Vec<InstantVector>>(data)?;
        Ok(())
    }

    #[test]
    fn test_range_vector_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
[
  {
    "metric": {
      "__name__": "up",
      "job": "prometheus",
      "instance": "localhost:9090"
    },
    "values": [
      [
        1435781430.781,
        "1"
      ],
      [
        1435781445.781,
        "1"
      ],
      [
        1435781460.781,
        "1"
      ]
    ]
  },
  {
    "metric": {
      "__name__": "up",
      "job": "node",
      "instance": "localhost:9091"
    },
    "values": [
      [
        1435781430.781,
        "0"
      ],
      [
        1435781445.781,
        "0"
      ],
      [
        1435781460.781,
        "1"
      ]
    ]
  }
]
"#;
        serde_json::from_str::<Vec<RangeVector>>(data)?;
        Ok(())
    }

    #[test]
    fn test_target_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
{
  "activeTargets": [
    {
      "discoveredLabels": {
        "__address__": "127.0.0.1:9090",
        "__metrics_path__": "/metrics",
        "__scheme__": "http",
        "job": "prometheus"
      },
      "labels": {
        "instance": "127.0.0.1:9090",
        "job": "prometheus"
      },
      "scrapePool": "prometheus",
      "scrapeUrl": "http://127.0.0.1:9090/metrics",
      "globalUrl": "http://example-prometheus:9090/metrics",
      "lastError": "",
      "lastScrape": "2017-01-17T15:07:44.723715405+01:00",
      "lastScrapeDuration": 0.050688943,
      "health": "up",
      "scrapeInterval": "1m",
      "scrapeTimeout": "10s"
    }
  ],
  "droppedTargets": [
    {
      "discoveredLabels": {
        "__address__": "127.0.0.1:9100",
        "__metrics_path__": "/metrics",
        "__scheme__": "http",
        "__scrape_interval__": "1m",
        "__scrape_timeout__": "10s",
        "job": "node"
      }
    }
  ]
}
"#;
        let targets = serde_json::from_str::<Targets>(data)?;
        let active = &targets.active();
        assert!(active.len() == 1);
        let target = &active[0];
        assert!(target
            .discovered_labels()
            .get("__address__")
            .is_some_and(|v| v == "127.0.0.1:9090"));
        assert!(target
            .discovered_labels()
            .get("__metrics_path__")
            .is_some_and(|v| v == "/metrics"));
        assert!(target
            .discovered_labels()
            .get("__scheme__")
            .is_some_and(|v| v == "http"));
        assert!(target
            .discovered_labels()
            .get("job")
            .is_some_and(|v| v == "prometheus"));
        assert!(target
            .labels()
            .get("instance")
            .is_some_and(|v| v == "127.0.0.1:9090"));
        assert!(target
            .labels()
            .get("job")
            .is_some_and(|v| v == "prometheus"));
        assert!(target.scrape_pool() == "prometheus");
        assert!(target.scrape_url() == &Url::parse("http://127.0.0.1:9090/metrics")?);
        assert!(target.global_url() == &Url::parse("http://example-prometheus:9090/metrics")?);
        assert!(target.last_error().is_empty());
        assert!(target.last_scrape() == &datetime!(2017-01-17 15:07:44.723715405 +1));
        assert!(target.last_scrape_duration() == 0.050688943);
        assert!(target.health().is_up());
        assert!(target.scrape_interval() == &Duration::seconds(60));
        assert!(target.scrape_timeout() == &Duration::seconds(10));
        let dropped = &targets.dropped();
        assert!(dropped.len() == 1);
        let target = &dropped[0];
        assert!(target
            .discovered_labels()
            .get("__address__")
            .is_some_and(|v| v == "127.0.0.1:9100"));
        assert!(target
            .discovered_labels()
            .get("__metrics_path__")
            .is_some_and(|v| v == "/metrics"));
        assert!(target
            .discovered_labels()
            .get("__scheme__")
            .is_some_and(|v| v == "http"));
        assert!(target
            .discovered_labels()
            .get("__scrape_interval__")
            .is_some_and(|v| v == "1m"));
        assert!(target
            .discovered_labels()
            .get("__scrape_timeout__")
            .is_some_and(|v| v == "10s"));
        assert!(target
            .discovered_labels()
            .get("job")
            .is_some_and(|v| v == "node"));
        Ok(())
    }

    #[test]
    fn test_rule_group_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
{
  "groups": [
    {
      "rules": [
        {
          "alerts": [
            {
              "activeAt": "2018-07-04T20:27:12.60602144+02:00",
              "annotations": {
                "summary": "High request latency"
              },
              "labels": {
                "alertname": "HighRequestLatency",
                "severity": "page"
              },
              "state": "firing",
              "value": "1e+00"
            }
          ],
          "annotations": {
            "summary": "High request latency"
          },
          "duration": 600,
          "health": "ok",
          "labels": {
            "severity": "page"
          },
          "name": "HighRequestLatency",
          "query": "job:request_latency_seconds:mean5m{job=\"myjob\"} > 0.5",
          "type": "alerting",
          "evaluationTime": 0.000312805,
          "lastEvaluation": "2023-10-05T19:51:25.462004334+02:00",
          "keepFiringFor": 60
        },
        {
          "health": "ok",
          "name": "job:http_inprogress_requests:sum",
          "query": "sum by (job) (http_inprogress_requests)",
          "type": "recording",
          "evaluationTime": 0.000256946,
          "lastEvaluation": "2023-10-05T19:51:25.052982522+02:00"
        }
      ],
      "file": "/rules.yaml",
      "interval": 60,
      "limit": 0,
      "name": "example",
      "evaluationTime": 0.000267716,
      "lastEvaluation": "2023-10-05T19:51:25.052974842+02:00"
    }
  ]
}
"#;
        let groups = serde_json::from_str::<RuleGroups>(data)?.groups;
        assert!(groups.len() == 1);
        let group = &groups[0];
        assert!(group.name() == "example");
        assert!(group.file() == "/rules.yaml");
        assert!(group.interval() == 60.0);
        assert!(group.limit() == 0);
        assert!(group.evaluation_time() == 0.000267716);
        assert!(group.last_evaluation() == &datetime!(2023-10-05 7:51:25.052974842 pm +2));
        assert!(group.rules().len() == 2);
        let alerting_rule = &group.rules[0].as_alerting().unwrap();
        assert!(alerting_rule.health() == RuleHealth::Good);
        assert!(alerting_rule.name() == "HighRequestLatency");
        assert!(alerting_rule.query() == "job:request_latency_seconds:mean5m{job=\"myjob\"} > 0.5");
        assert!(alerting_rule.evaluation_time() == 0.000312805);
        assert!(alerting_rule.last_evaluation() == &datetime!(2023-10-05 7:51:25.462004334 pm +2));
        assert!(alerting_rule.duration() == 600.0);
        assert!(alerting_rule.keep_firing_for() == 60.0);
        assert!(alerting_rule.alerts().len() == 1);
        assert!(alerting_rule
            .annotations()
            .get("summary")
            .is_some_and(|v| v == "High request latency"));
        let alert = &alerting_rule.alerts()[0];
        assert!(alert.value() == 1.0);
        assert!(alert.state().is_firing());
        assert!(alert.active_at() == &datetime!(2018-07-04 20:27:12.60602144 +2));
        let recording_rule = &group.rules[1].as_recording().unwrap();
        assert!(recording_rule.health() == RuleHealth::Good);
        assert!(recording_rule.name() == "job:http_inprogress_requests:sum");
        assert!(recording_rule.query() == "sum by (job) (http_inprogress_requests)");
        assert!(recording_rule.evaluation_time() == 0.000256946);
        assert!(recording_rule.last_evaluation() == &datetime!(2023-10-05 7:51:25.052982522 pm +2));
        Ok(())
    }

    #[test]
    fn test_alert_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
{
  "alerts": [
     {
        "activeAt":"2018-07-04T20:27:12.60602144+02:00",
        "annotations":{
        },
        "labels":{
           "alertname":"my-alert"
        },
        "state":"firing",
        "value":"1e+00"
     }
  ]
}
"#;
        serde_json::from_str::<Alerts>(data)?;
        Ok(())
    }

    #[test]
    fn test_target_metadata_deserialization_1() -> Result<(), anyhow::Error> {
        let data = r#"
[
  {
    "target": {
      "instance": "127.0.0.1:9090",
      "job": "prometheus"
    },
    "type": "gauge",
    "help": "Number of goroutines that currently exist.",
    "unit": ""
  },
  {
    "target": {
      "instance": "127.0.0.1:9091",
      "job": "prometheus"
    },
    "type": "gauge",
    "help": "Number of goroutines that currently exist.",
    "unit": ""
  },
  {
    "target": {
      "instance": "localhost:9090",
      "job": "prometheus"
    },
    "metric": "process_virtual_memory_bytes",
    "type": "gauge",
    "help": "Virtual memory size in bytes.",
    "unit": ""
  },
  {
    "target": {
      "instance": "localhost:9090",
      "job": "prometheus"
    },
    "metric": "prometheus_http_response_size_bytes",
    "type": "histogram",
    "help": "Histogram of response size for HTTP requests.",
    "unit": ""
  },
  {
    "target": {
      "instance": "localhost:9090",
      "job": "prometheus"
    },
    "metric": "prometheus_ready",
    "type": "gauge",
    "help": "Whether Prometheus startup was fully completed and the server is ready for normal operation.",
    "unit": ""
  },
  {
    "target": {
      "instance": "localhost:9090",
      "job": "prometheus"
    },
    "metric": "prometheus_rule_group_iterations_missed_total",
    "type": "counter",
    "help": "The total number of rule group evaluations missed due to slow rule group evaluation.",
    "unit": ""
  },
  {
    "target": {
      "instance": "localhost:9090",
      "job": "prometheus"
    },
    "metric": "prometheus_target_scrape_pool_reloads_failed_total",
    "type": "counter",
    "help": "Total number of failed scrape pool reloads.",
    "unit": ""
  },
  {
    "target": {
      "instance": "localhost:9090",
      "job": "prometheus"
    },
    "metric": "prometheus_target_scrape_pool_reloads_total",
    "type": "counter",
    "help": "Total number of scrape pool reloads.",
    "unit": ""
  }
]
"#;
        let metadata = serde_json::from_str::<Vec<TargetMetadata>>(data)?;
        assert!(metadata.len() == 8);
        let first = &metadata[0];
        assert!(first
            .target()
            .get("instance")
            .is_some_and(|v| v == "127.0.0.1:9090"));
        assert!(first.target().get("job").is_some_and(|v| v == "prometheus"));
        assert!(first.metric_type().is_gauge());
        assert!(first.help() == "Number of goroutines that currently exist.");
        assert!(first.unit().is_empty());
        assert!(first.metric().is_none());
        let third = &metadata[2];
        assert!(third
            .target()
            .get("instance")
            .is_some_and(|v| v == "localhost:9090"));
        assert!(third.target().get("job").is_some_and(|v| v == "prometheus"));
        assert!(third.metric_type().is_gauge());
        assert!(third.help() == "Virtual memory size in bytes.");
        assert!(third.unit().is_empty());
        assert!(third
            .metric()
            .is_some_and(|v| v == "process_virtual_memory_bytes"));
        let fourth = &metadata[3];
        assert!(fourth
            .target()
            .get("instance")
            .is_some_and(|v| v == "localhost:9090"));
        assert!(fourth
            .target()
            .get("job")
            .is_some_and(|v| v == "prometheus"));
        assert!(fourth.metric_type().is_histogram());
        assert!(fourth.help() == "Histogram of response size for HTTP requests.");
        assert!(fourth.unit().is_empty());
        assert!(fourth
            .metric()
            .is_some_and(|v| v == "prometheus_http_response_size_bytes"));
        Ok(())
    }

    #[test]
    fn test_target_metadata_deserialization_2() -> Result<(), anyhow::Error> {
        let data = r#"
[
  {
    "target": {
      "instance": "127.0.0.1:9090",
      "job": "prometheus"
    },
    "metric": "prometheus_treecache_zookeeper_failures_total",
    "type": "counter",
    "help": "The total number of ZooKeeper failures.",
    "unit": ""
  },
  {
    "target": {
      "instance": "127.0.0.1:9090",
      "job": "prometheus"
    },
    "metric": "prometheus_tsdb_reloads_total",
    "type": "counter",
    "help": "Number of times the database reloaded block data from disk.",
    "unit": ""
  }
]
"#;
        serde_json::from_str::<Vec<TargetMetadata>>(data)?;
        Ok(())
    }

    #[test]
    fn test_metric_metadata_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
{
  "cortex_ring_tokens": [
    {
      "type": "gauge",
      "help": "Number of tokens in the ring",
      "unit": ""
    }
  ],
  "http_requests_total": [
    {
      "type": "counter",
      "help": "Number of HTTP requests",
      "unit": ""
    },
    {
      "type": "counter",
      "help": "Amount of HTTP requests",
      "unit": ""
    }
  ]
}
"#;
        let metadata = serde_json::from_str::<HashMap<String, Vec<MetricMetadata>>>(data)?;
        assert!(metadata.len() == 2);
        assert!(metadata
            .get("cortex_ring_tokens")
            .is_some_and(|v| v[0].metric_type().is_gauge()
                && v[0].help() == "Number of tokens in the ring"
                && v[0].unit().is_empty()));
        assert!(metadata.get("http_requests_total").is_some_and(|v| v[0]
            .metric_type()
            .is_counter()
            && v[0].help() == "Number of HTTP requests"
            && v[0].unit().is_empty()));
        Ok(())
    }

    #[test]
    fn test_alertmanagers_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
{
  "activeAlertmanagers": [
    {
      "url": "http://127.0.0.1:9090/api/v1/alerts"
    }
  ],
  "droppedAlertmanagers": [
    {
      "url": "http://127.0.0.1:9093/api/v1/alerts"
    }
  ]
}
"#;
        serde_json::from_str::<Alertmanagers>(data)?;
        Ok(())
    }

    #[test]
    fn test_buildinformation_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
{
  "version": "2.13.1",
  "revision": "cb7cbad5f9a2823a622aaa668833ca04f50a0ea7",
  "branch": "master",
  "buildUser": "julius@desktop",
  "buildDate": "20191102-16:19:51",
  "goVersion": "go1.13.1"
}
"#;
        serde_json::from_str::<BuildInformation>(data)?;
        Ok(())
    }

    #[test]
    fn test_runtimeinformation_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
{
  "startTime": "2019-11-02T17:23:59.301361365+01:00",
  "CWD": "/",
  "reloadConfigSuccess": true,
  "lastConfigTime": "2019-11-02T17:23:59+01:00",
  "timeSeriesCount": 873,
  "corruptionCount": 0,
  "goroutineCount": 48,
  "GOMAXPROCS": 4,
  "GOGC": "",
  "GODEBUG": "",
  "storageRetention": "15d"
}
"#;
        serde_json::from_str::<RuntimeInformation>(data)?;
        Ok(())
    }

    #[test]
    fn test_tsdb_stats_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
{
  "headStats": {
    "numSeries": 508,
    "chunkCount": 937,
    "minTime": 1591516800000,
    "maxTime": 1598896800143
  },
  "seriesCountByMetricName": [
    {
      "name": "net_conntrack_dialer_conn_failed_total",
      "value": 20
    },
    {
      "name": "prometheus_http_request_duration_seconds_bucket",
      "value": 20
    }
  ],
  "labelValueCountByLabelName": [
    {
      "name": "__name__",
      "value": 211
    },
    {
      "name": "event",
      "value": 3
    }
  ],
  "memoryInBytesByLabelName": [
    {
      "name": "__name__",
      "value": 8266
    },
    {
      "name": "instance",
      "value": 28
    }
  ],
  "seriesCountByLabelValuePair": [
    {
      "name": "job=prometheus",
      "value": 425
    },
    {
      "name": "instance=localhost:9090",
      "value": 425
    }
  ]
}
"#;
        serde_json::from_str::<TsdbStatistics>(data)?;
        Ok(())
    }

    #[test]
    fn test_wal_replay_deserialization() -> Result<(), anyhow::Error> {
        let data = r#"
{
  "min": 2,
  "max": 5,
  "current": 40,
  "state": "waiting"
}
"#;
        let result: Result<WalReplayStatistics, serde_json::Error> = serde_json::from_str(data);
        assert!(result.is_ok());

        let data = r#"
{
  "min": 2,
  "max": 5,
  "current": 40,
  "state": "in progress"
}
"#;
        let result: Result<WalReplayStatistics, serde_json::Error> = serde_json::from_str(data);
        assert!(result.is_ok());

        let data = r#"
{
  "min": 2,
  "max": 5,
  "current": 40,
  "state": "done"
}
"#;
        let result: Result<WalReplayStatistics, serde_json::Error> = serde_json::from_str(data);
        assert!(result.is_ok());

        let data = r#"
{
  "min": 2,
  "max": 5,
  "current": 40
}
"#;
        serde_json::from_str::<WalReplayStatistics>(data)?;
        Ok(())
    }
}
