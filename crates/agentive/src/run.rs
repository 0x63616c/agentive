//! Local execution façade and the public agent runtime API.

mod agent;
mod effects;
mod handle;
mod local;
mod provider;
mod types;
mod validation;

pub use agent::{Agent, AgentBuilder};
pub use handle::RunHandle;
pub use provider::DynModelProvider;
pub use types::{
    AgentEffectOutcome, RunEvent, RunOptions, RunRecord, RunResult, RunStatus, ToolEffectResult,
};

use effects::execute_effect;
pub(crate) use handle::RunObserver;
use handle::{AtomicRunStatus, EventBus, emit_event};
use local::{RunExecution, execute_run, remaining_time};
use provider::{ProviderErased, execute_provider_effect};

pub(super) use validation::{validate_context, validate_history};
