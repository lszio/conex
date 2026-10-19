//! Local installed contract metadata; no remote schema loading or domain logic.
use crate::{
    Error, Result,
    state::{StateLease, StateObserver},
};
use conex_proto::{ErrorCode, RequestContext};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};

pub type HandlerFuture<T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'static>>;
pub type InvokeHandler =
    Arc<dyn Fn(InvocationContext, Value) -> HandlerFuture<Value> + Send + Sync>;
pub type StateHandler =
    Arc<dyn Fn(InvocationContext, Value, StateObserver) -> HandlerFuture<StateLease> + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Execution {
    ReadOnly,
    NonReplayable,
}
impl Execution {
    pub(crate) fn uncertain(self, code: ErrorCode, message: impl Into<String>) -> Error {
        match self {
            Self::NonReplayable => {
                Error::new(ErrorCode::OutcomeUnknown, message, "unknown", "never")
            }
            Self::ReadOnly => Error::new(code, message, "unknown", "safe"),
        }
    }
}

/// Cooperative only: signalling cancellation never proves a side effect stopped.
#[derive(Clone, Debug)]
pub struct Cancellation(pub(crate) tokio::sync::watch::Receiver<bool>);
impl Cancellation {
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }
    pub async fn cancelled(&mut self) {
        while !*self.0.borrow_and_update() {
            if self.0.changed().await.is_err() {
                return;
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct InvocationContext {
    /// Authenticated hub link identity, not supplied by the business input.
    pub hub: conex_proto::LinkIdentity,
    pub caller: RequestContext,
    pub request_id: String,
    pub cancellation: Cancellation,
}

#[derive(Clone)]
pub struct Method {
    pub input_schema: String,
    pub output_schema: String,
    pub execution: Execution,
    pub(crate) handler: Option<InvokeHandler>,
}
impl Method {
    pub fn new(
        input_schema: impl Into<String>,
        output_schema: impl Into<String>,
        execution: Execution,
    ) -> Self {
        Self {
            input_schema: input_schema.into(),
            output_schema: output_schema.into(),
            execution,
            handler: None,
        }
    }
    /// `I`/`O` should be the generated domain messages matching the schema names.
    pub fn with_handler<I, O, F, Fut>(mut self, handler: F) -> Self
    where
        I: DeserializeOwned + Send + 'static,
        O: Serialize + Send + 'static,
        F: Fn(InvocationContext, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<O>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        self.handler = Some(Arc::new(move |context, value| {
            let handler = handler.clone();
            Box::pin(async move {
                let input = serde_json::from_value(value).map_err(Error::decode)?;
                let output = handler(context, input).await?;
                serde_json::to_value(output).map_err(Error::decode)
            })
        }));
        self
    }
}
#[derive(Clone)]
pub struct State {
    pub input_schema: String,
    pub value_schema: String,
    pub(crate) handler: Option<StateHandler>,
}
impl State {
    pub fn new(input_schema: impl Into<String>, value_schema: impl Into<String>) -> Self {
        Self {
            input_schema: input_schema.into(),
            value_schema: value_schema.into(),
            handler: None,
        }
    }
    /// Attach the supplied observer and capture the snapshot under the SAME domain lock.
    /// The returned lease owns the observer registration and removes it on Drop.
    pub fn with_handler<I, F, Fut>(mut self, handler: F) -> Self
    where
        I: DeserializeOwned + Send + 'static,
        F: Fn(InvocationContext, I, StateObserver) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<StateLease>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        self.handler = Some(Arc::new(move |context, value, observer| {
            let handler = handler.clone();
            Box::pin(async move {
                let input = serde_json::from_value(value).map_err(Error::decode)?;
                handler(context, input, observer).await
            })
        }));
        self
    }
}

#[derive(Clone)]
pub struct Contract {
    pub id: String,
    pub methods: BTreeMap<String, Method>,
    pub states: BTreeMap<String, State>,
}
impl Contract {
    pub fn new(
        id: impl Into<String>,
        methods: BTreeMap<String, Method>,
        states: BTreeMap<String, State>,
    ) -> Result<Self> {
        let id = id.into();
        if id.is_empty()
            || (methods.is_empty() && states.is_empty())
            || methods.keys().chain(states.keys()).any(String::is_empty)
        {
            return Err(Error::bad_request("empty contract or member name"));
        }
        Ok(Self {
            id,
            methods,
            states,
        })
    }
}

/// Fixed registration binding. No transparent rebinding after reconnect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleRef {
    pub space_id: String,
    pub provider_endpoint_id: String,
    pub registration_id: String,
    pub contract_id: String,
}
impl From<&conex_proto::ModuleInfo> for ModuleRef {
    fn from(m: &conex_proto::ModuleInfo) -> Self {
        Self {
            space_id: m.space_id.clone(),
            provider_endpoint_id: m.provider_endpoint_id.clone(),
            registration_id: m.registration_id.clone(),
            contract_id: m.contract_id.clone(),
        }
    }
}
impl From<&conex_proto::ModuleRegisterResult> for ModuleRef {
    fn from(m: &conex_proto::ModuleRegisterResult) -> Self {
        Self {
            space_id: m.space_id.clone(),
            provider_endpoint_id: m.provider_endpoint_id.clone(),
            registration_id: m.registration_id.clone(),
            contract_id: m.contract_id.clone(),
        }
    }
}
impl ModuleRef {
    pub(crate) fn context(&self) -> RequestContext {
        RequestContext {
            plane: conex_proto::Plane::Broker as i32,
            space_id: self.space_id.clone(),
            provider_endpoint_id: self.provider_endpoint_id.clone(),
            registration_id: self.registration_id.clone(),
            ..Default::default()
        }
    }
    pub(crate) fn matches(&self, c: &RequestContext) -> bool {
        self.space_id == c.space_id
            && self.provider_endpoint_id == c.provider_endpoint_id
            && self.registration_id == c.registration_id
    }
}

/// Explicit local upper bound, intersected with the installed contract and registration.
/// Empty sets deny access; identity is accepted only from the authenticated hub.
#[derive(Clone, Debug, Default)]
pub struct Exposure {
    pub principals: BTreeSet<String>,
    pub tenants: BTreeSet<String>,
    pub methods: BTreeSet<String>,
    pub states: BTreeSet<String>,
}
impl Exposure {
    pub(crate) fn allows(&self, context: &RequestContext, member: &str, state: bool) -> bool {
        self.principals.contains(&context.principal_id)
            && self.tenants.contains(&context.tenant_id)
            && if state {
                self.states.contains(member)
            } else {
                self.methods.contains(member)
            }
    }
}
#[derive(Clone)]
pub(crate) struct Published {
    pub target: ModuleRef,
    pub contract: Arc<Contract>,
    pub exposure: Exposure,
}

pub(crate) fn validate(schema: &str, value: &Value) -> Result<()> {
    conex_proto::capability::validate_domain_schema(schema, value).map_err(Error::protocol)
}
