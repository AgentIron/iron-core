//! Tests for the caller-owned config bridge: source projection, snapshot
//! isolation, validation failures, and backward compatibility.
#![allow(deprecated)]

use futures::stream::{self, BoxStream};
use futures::StreamExt;
use iron_core::{
    config::{ApprovalStrategy, ConfigSource},
    Config, LoopError, Provider, ProviderEvent, Session, SessionHandle, ToolPolicy,
};
use iron_providers::{GenerationConfig, InferenceRequest, OpenAiConfig, OpenAiProvider};

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

struct MockProvider {
    stream_responses: Arc<Mutex<VecDeque<Vec<ProviderEvent>>>>,
    requests: Arc<Mutex<Vec<InferenceRequest>>>,
}

impl MockProvider {
    fn with_stream_responses(responses: Vec<Vec<ProviderEvent>>) -> Self {
        Self {
            stream_responses: Arc::new(Mutex::new(responses.into())),
            requests: Arc::new(Mutex::new(vec![])),
        }
    }
}

impl Provider for MockProvider {
    fn infer(
        &self,
        request: InferenceRequest,
    ) -> iron_providers::ProviderFuture<'_, Vec<ProviderEvent>> {
        self.requests.lock().unwrap().push(request);
        let response = self
            .stream_responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| vec![ProviderEvent::Complete]);
        Box::pin(async move { Ok(response) })
    }

    fn infer_stream(
        &self,
        request: InferenceRequest,
    ) -> iron_providers::ProviderFuture<
        '_,
        BoxStream<'static, iron_providers::ProviderResult<ProviderEvent>>,
    > {
        self.requests.lock().unwrap().push(request);
        let response = self
            .stream_responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| vec![ProviderEvent::Complete]);
        Box::pin(async move { Ok(stream::iter(response.into_iter().map(Ok)).boxed()) })
    }
}

struct AppConfig {
    model: String,
    temperature: f32,
    max_iterations: u32,
}

impl AppConfig {
    fn new(model: &str, temperature: f32) -> Self {
        Self {
            model: model.to_string(),
            temperature,
            max_iterations: 10,
        }
    }
}

impl ConfigSource for AppConfig {
    fn to_config(&self) -> Result<Config, LoopError> {
        Ok(Config::new()
            .with_model(&self.model)
            .with_max_iterations(self.max_iterations)
            .with_default_generation(GenerationConfig::new().with_temperature(self.temperature)))
    }
}

async fn collect_finished(events: &mut iron_core::TurnEvents) -> Vec<iron_core::TurnEvent> {
    let mut collected = Vec::new();
    while let Some(event) = events.next_event().await {
        let is_finished = matches!(event, iron_core::TurnEvent::Finished { .. });
        collected.push(event);
        if is_finished {
            break;
        }
    }
    collected
}

#[tokio::test]
async fn snapshot_isolation_config_mutation_after_construction() {
    let mut app_config = AppConfig::new("gpt-4o", 0.7);
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::Output {
            content: "hello".into(),
        },
        ProviderEvent::Complete,
    ]]);

    let handle = SessionHandle::from_source(&app_config, provider, Session::new()).unwrap();
    let config_snapshot = handle.config().clone();
    assert_eq!(config_snapshot.model, "gpt-4o");
    assert_eq!(config_snapshot.default_generation.temperature, Some(0.7));

    app_config.model = "gpt-3.5-turbo".to_string();
    app_config.temperature = 0.1;

    let config_after = handle.config().clone();
    assert_eq!(config_after.model, "gpt-4o");
    assert_eq!(config_after.default_generation.temperature, Some(0.7));
}

#[tokio::test]
async fn snapshot_isolation_mutation_does_not_affect_turns() {
    let mut app_config = AppConfig::new("gpt-4o", 0.5);
    let provider = MockProvider::with_stream_responses(vec![vec![ProviderEvent::Complete]]);

    let handle = SessionHandle::from_source(&app_config, provider, Session::new()).unwrap();
    let (_, mut events) = handle.start_turn("hi").unwrap();
    collect_finished(&mut events).await;

    let requests = handle.config();
    assert_eq!(requests.model, "gpt-4o");
    assert_eq!(requests.default_generation.temperature, Some(0.5));

    app_config.model = "claude-3".to_string();
    app_config.temperature = 0.1;
}

#[test]
fn source_trait_projection_produces_valid_config() {
    let app_config = AppConfig::new("gpt-4o-mini", 0.3);
    let config = app_config.to_config().unwrap();
    assert_eq!(config.model, "gpt-4o-mini");
    assert_eq!(config.default_generation.temperature, Some(0.3));
    assert_eq!(config.max_iterations, 10);
}

#[tokio::test]
async fn bridge_construction_success() {
    let app_config = AppConfig::new("gpt-4o", 0.5);
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::Output {
            content: "response".into(),
        },
        ProviderEvent::Complete,
    ]]);

    let handle = SessionHandle::from_source(&app_config, provider, Session::new()).unwrap();
    let (_, mut events) = handle.start_turn("hello").unwrap();
    let all = collect_finished(&mut events).await;

    assert!(all.iter().any(
        |e| matches!(e, iron_core::TurnEvent::OutputDelta { content } if content == "response")
    ));
}

#[tokio::test]
async fn bridge_construction_fails_empty_model() {
    struct EmptyModelConfig;
    impl ConfigSource for EmptyModelConfig {
        fn to_config(&self) -> Result<Config, LoopError> {
            Ok(Config::new().with_model(""))
        }
    }

    let provider = MockProvider::with_stream_responses(vec![]);
    let result = SessionHandle::from_source(&EmptyModelConfig, provider, Session::new());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, LoopError::InvalidConfig { .. }));
    assert!(err.to_string().contains("model"));
}

#[tokio::test]
async fn bridge_construction_fails_zero_max_iterations() {
    struct ZeroIterConfig;
    impl ConfigSource for ZeroIterConfig {
        fn to_config(&self) -> Result<Config, LoopError> {
            Ok(Config::new().with_max_iterations(0))
        }
    }

    let provider = MockProvider::with_stream_responses(vec![]);
    let result = SessionHandle::from_source(&ZeroIterConfig, provider, Session::new());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, LoopError::InvalidConfig { .. }));
    assert!(err.to_string().contains("max_iterations"));
}

#[tokio::test]
async fn bridge_construction_fails_invalid_temperature() {
    struct BadTempConfig;
    impl ConfigSource for BadTempConfig {
        fn to_config(&self) -> Result<Config, LoopError> {
            Ok(
                Config::new()
                    .with_default_generation(GenerationConfig::new().with_temperature(5.0)),
            )
        }
    }

    let provider = MockProvider::with_stream_responses(vec![]);
    let result = SessionHandle::from_source(&BadTempConfig, provider, Session::new());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, LoopError::InvalidConfig { .. }));
    assert!(err.to_string().contains("temperature"));
}

#[test]
fn openai_config_source_empty_key_fails() {
    struct EmptyKeySource;
    impl iron_providers::OpenAiConfigSource for EmptyKeySource {
        fn to_openai_config(&self) -> Result<OpenAiConfig, iron_providers::ProviderError> {
            Ok(OpenAiConfig::new("".to_string()))
        }
    }

    let result = OpenAiProvider::from_source(&EmptyKeySource);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("API key"));
}

#[test]
fn runtime_config_source_empty_key_fails() {
    use iron_providers::{
        ApiFamily, GenericProvider, ProviderProfile, RuntimeConfig, RuntimeConfigSource,
    };

    struct EmptyKeySource;
    impl RuntimeConfigSource for EmptyKeySource {
        fn to_runtime_config(&self) -> Result<RuntimeConfig, iron_providers::ProviderError> {
            Ok(RuntimeConfig::new(""))
        }
    }

    let profile = ProviderProfile::new(
        "test",
        ApiFamily::OpenAiResponses,
        "https://api.example.com",
    );
    let result = GenericProvider::from_source(profile, &EmptyKeySource);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("API key"));
}

#[tokio::test]
async fn regression_direct_construction_still_works() {
    let provider = MockProvider::with_stream_responses(vec![vec![
        ProviderEvent::Output {
            content: "direct".into(),
        },
        ProviderEvent::Complete,
    ]]);
    let config = Config::new()
        .with_model("gpt-4o")
        .with_max_iterations(5)
        .with_approval_strategy(ApprovalStrategy::Never)
        .with_default_generation(GenerationConfig::new().with_temperature(0.8));

    let handle = SessionHandle::new(config.clone(), provider, Session::new());
    assert_eq!(handle.config().model, "gpt-4o");
    assert_eq!(handle.config().max_iterations, 5);
    assert_eq!(
        handle.config().default_approval_strategy,
        ApprovalStrategy::Never
    );
    assert_eq!(handle.config().default_generation.temperature, Some(0.8));

    let (_, mut events) = handle.start_turn("hi").unwrap();
    let all = collect_finished(&mut events).await;
    assert!(all.iter().any(
        |e| matches!(e, iron_core::TurnEvent::OutputDelta { content } if content == "direct")
    ));
}

#[tokio::test]
async fn regression_direct_openai_construction_still_works() {
    let config = OpenAiConfig::new("sk-test".to_string()).with_model("gpt-4o".to_string());
    let _provider = OpenAiProvider::new(config);
}

#[test]
fn regression_direct_config_construction_no_source_trait() {
    let config = Config::default()
        .with_model("my-model")
        .with_max_iterations(20)
        .with_default_tool_policy(ToolPolicy::Required);

    assert_eq!(config.model, "my-model");
    assert_eq!(config.max_iterations, 20);
    assert_eq!(config.default_tool_policy, ToolPolicy::Required);
}
