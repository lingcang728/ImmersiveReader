use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Podcast,
    Zhihu,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Queued,
    Starting,
    Running,
    Pausing,
    Paused,
    Stopping,
    Terminal,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    None,
    Success,
    PartialSuccess,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequiredAction {
    None,
    Login,
    Captcha,
    ConfigureSecret,
    FreeDiskSpace,
    ApproveBudget,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskErrorCode {
    LoginRequired,
    CaptchaRequired,
    SecretMissing,
    InsufficientDisk,
    BudgetConfirmationRequired,
    InputChanged,
    InputCopyFailed,
    PipelineIncompatible,
    ModelIncompatible,
    ConfigIncompatible,
    EngineUnavailable,
    EngineProtocolMismatch,
    EngineCrashed,
    UpstreamUnauthorized,
    RateLimited,
    UpstreamTimeout,
    UpstreamUnavailable,
    PublishFailed,
    PublishRecoveryRequired,
    MigrationRequired,
    CancelledByUser,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressMode {
    Indeterminate,
    Determinate,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskProgress {
    pub mode: ProgressMode,
    pub percent: Option<f64>,
    pub completed_units: Option<u64>,
    pub total_units: Option<u64>,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSnapshot {
    pub id: String,
    pub kind: TaskKind,
    pub revision: u64,
    pub last_sequence: u64,
    pub lifecycle_state: LifecycleState,
    pub outcome: TaskOutcome,
    pub required_action: RequiredAction,
    pub progress: TaskProgress,
    pub error_code: Option<TaskErrorCode>,
    pub error_message: Option<String>,
    #[serde(default)]
    pub retry_after_seconds: Option<u64>,
    pub engine_stage: String,
    pub engine_status: String,
    pub recoverable: bool,
    pub can_pause: bool,
    pub can_resume: bool,
    pub can_retry: bool,
    pub can_cancel: bool,
    pub book_id: Option<String>,
    pub source_id: Option<String>,
    pub cache_lease_bytes: u64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEvent {
    pub schema_version: u32,
    pub task_id: String,
    pub sequence: u64,
    pub revision: u64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub snapshot: TaskSnapshot,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcquisitionSnapshot {
    pub tasks: Vec<TaskSnapshot>,
    pub recoverable_cache_bytes: u64,
    pub generated_at: String,
}
