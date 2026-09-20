use std::collections::HashMap;
use std::sync::Arc;

use titi_engine::{
    EngineCommand, EngineConfig, EngineEvent, EngineRuntime, RegistryError, ResolvedModel,
    TransportResolver,
};
use titi_providers::{
    BlockId, ChatMessage, MockBody, MockTransport, Role, StopReason, StreamEvent, ToolCallRef,
    Transport, TransportError,
};

struct MapResolver(HashMap<String, Arc<dyn Transport>>);

impl TransportResolver for MapResolver {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, RegistryError> {
        self.0
            .get(model)
            .cloned()
            .map(|transport| ResolvedModel::without_credential(model, transport))
            .ok_or_else(|| RegistryError::UnknownModel(model.into()))
    }
}

fn resolver(entries: Vec<(&str, Arc<dyn Transport>)>) -> Arc<dyn TransportResolver> {
    Arc::new(MapResolver(
        entries
            .into_iter()
            .map(|(model, transport)| (model.to_owned(), transport))
            .collect(),
    ))
}

async fn collect_until_terminal(engine: &mut titi_engine::Engine) -> Vec<EngineEvent> {
    let mut events = Vec::new();
    while let Some(event) = engine.recv().await {
        let terminal = matches!(
            event,
            EngineEvent::TurnFinished { .. }
                | EngineEvent::Failed { .. }
                | EngineEvent::Cancelled { .. }
        );
        events.push(event);
        if terminal {
            break;
        }
    }
    events
}

#[tokio::test]
async fn streams_prompt_to_completion() {
    let transport = Arc::new(MockTransport::new(vec![MockBody::Events(vec![
        StreamEvent::Start,
        StreamEvent::TextDelta {
            id: BlockId::new("text"),
            text: "hello".into(),
        },
        StreamEvent::Done {
            reason: StopReason::Stop,
        },
    ])]));
    let mut engine = EngineRuntime::start(
        EngineConfig::new("primary"),
        resolver(vec![("primary", transport)]),
    );

    engine
        .send(EngineCommand::SubmitPrompt { text: "hi".into() })
        .await
        .unwrap();
    let events = collect_until_terminal(&mut engine).await;

    assert!(
        events
            .iter()
            .any(|event| matches!(event, EngineEvent::StreamDelta { text, .. } if text == "hello"))
    );
    assert!(matches!(
        events.last(),
        Some(EngineEvent::TurnFinished {
            reason: StopReason::Stop,
            ..
        })
    ));
}

fn workspace_with_hub_and_leaf() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/hub.rs"), "pub fn hub() {}\n").unwrap();
    std::fs::write(dir.path().join("src/leaf.rs"), "pub fn leaf() {}\n").unwrap();
    dir
}

#[tokio::test]
async fn genome_is_indexed_and_injected_as_system_message() {
    let workspace = workspace_with_hub_and_leaf();
    let transport = Arc::new(MockTransport::new(vec![MockBody::Events(vec![
        StreamEvent::TextDelta {
            id: BlockId::new("text"),
            text: "ok".into(),
        },
        StreamEvent::Done {
            reason: StopReason::Stop,
        },
    ])]));
    let mut config = EngineConfig::new("primary");
    config.genome_root = Some(workspace.path().to_path_buf());
    let mut engine = EngineRuntime::start(
        config,
        resolver(vec![("primary", Arc::clone(&transport) as _)]),
    );

    engine
        .send(EngineCommand::SubmitPrompt { text: "hi".into() })
        .await
        .unwrap();
    let _ = collect_until_terminal(&mut engine).await;

    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    let messages = &requests[0].messages;
    assert_eq!(messages.len(), 2, "system + user");
    assert_eq!(messages[0].role, Role::System);
    assert!(
        messages[0].content.starts_with("<genome>\n"),
        "{}",
        messages[0].content
    );
    assert!(messages[0].content.contains("src/hub.rs"));
    assert!(messages[0].content.contains("src/leaf.rs"));
    assert_eq!(messages[1].role, Role::User);
    assert_eq!(messages[1].content, "hi");
}

#[tokio::test]
async fn genome_refreshes_between_turns() {
    let workspace = workspace_with_hub_and_leaf();
    let transport = Arc::new(MockTransport::new(vec![
        MockBody::Events(vec![StreamEvent::Done {
            reason: StopReason::Stop,
        }]),
        MockBody::Events(vec![StreamEvent::Done {
            reason: StopReason::Stop,
        }]),
    ]));
    let mut config = EngineConfig::new("primary");
    config.genome_root = Some(workspace.path().to_path_buf());
    let mut engine = EngineRuntime::start(
        config,
        resolver(vec![("primary", Arc::clone(&transport) as _)]),
    );

    engine
        .send(EngineCommand::SubmitPrompt { text: "one".into() })
        .await
        .unwrap();
    let _ = collect_until_terminal(&mut engine).await;

    // A file created after startup is in the next turn's map.
    std::fs::write(workspace.path().join("src/fresh.rs"), "pub fn fresh() {}\n").unwrap();
    engine
        .send(EngineCommand::SubmitPrompt { text: "two".into() })
        .await
        .unwrap();
    let _ = collect_until_terminal(&mut engine).await;

    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert!(!requests[0].messages[0].content.contains("src/fresh.rs"));
    assert!(
        requests[1].messages[0].content.contains("src/fresh.rs"),
        "second turn must see the new file: {}",
        requests[1].messages[0].content
    );
}

#[tokio::test]
async fn no_genome_means_prompt_only() {
    let transport = Arc::new(MockTransport::new(vec![MockBody::Events(vec![
        StreamEvent::Done {
            reason: StopReason::Stop,
        },
    ])]));
    let mut engine = EngineRuntime::start(
        EngineConfig::new("primary"),
        resolver(vec![("primary", Arc::clone(&transport) as _)]),
    );
    engine
        .send(EngineCommand::SubmitPrompt { text: "hi".into() })
        .await
        .unwrap();
    let _ = collect_until_terminal(&mut engine).await;

    let requests = transport.requests();
    assert_eq!(requests[0].messages.len(), 1);
    assert_eq!(requests[0].messages[0].role, Role::User);
}

#[tokio::test]
async fn touched_file_leads_the_next_projection() {
    use titi_tools::{ApprovalMode, EchoTool, ReadFileTool, ToolRegistry};

    let workspace = workspace_with_hub_and_leaf();
    let transport = Arc::new(MockTransport::new(vec![
        MockBody::Events(vec![
            StreamEvent::ToolcallStart {
                id: BlockId::new("tool"),
                call: ToolCallRef {
                    call_id: "call-1".into(),
                    name: "read".into(),
                },
            },
            StreamEvent::ToolcallDelta {
                id: BlockId::new("tool"),
                json: r#"{"path":"src/leaf.rs"}"#.into(),
            },
            StreamEvent::ToolcallEnd {
                id: BlockId::new("tool"),
            },
            StreamEvent::Done {
                reason: StopReason::ToolUse,
            },
        ]),
        MockBody::Events(vec![StreamEvent::Done {
            reason: StopReason::Stop,
        }]),
        MockBody::Events(vec![StreamEvent::Done {
            reason: StopReason::Stop,
        }]),
    ]));
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(ReadFileTool {
        root: workspace.path().to_path_buf(),
    }));
    tools.register(Arc::new(EchoTool));

    let mut config = EngineConfig::new("primary");
    config.genome_root = Some(workspace.path().to_path_buf());
    config.approval_mode = ApprovalMode::Yolo;
    let mut engine = EngineRuntime::start_with_tools(
        config,
        resolver(vec![("primary", Arc::clone(&transport) as _)]),
        tools,
    );

    // Turn 1 reads src/leaf.rs; turn 2 should lead with it.
    engine
        .send(EngineCommand::SubmitPrompt {
            text: "read leaf".into(),
        })
        .await
        .unwrap();
    let _ = collect_until_terminal(&mut engine).await;
    engine
        .send(EngineCommand::SubmitPrompt {
            text: "again".into(),
        })
        .await
        .unwrap();
    let _ = collect_until_terminal(&mut engine).await;

    let requests = transport.requests();
    let second = requests
        .iter()
        .find(|request| {
            request.messages.first().is_some_and(|message| {
                message.role == Role::System && message.content.contains("src/leaf.rs")
            }) && request
                .messages
                .iter()
                .any(|message| message.content == "again")
        })
        .expect("second turn reached the provider");
    let system = &second.messages[0].content;
    let leaf_at = system.find("src/leaf.rs").unwrap();
    let hub_at = system.find("src/hub.rs").unwrap();
    assert!(
        leaf_at < hub_at,
        "touched file must lead the map:\n{system}"
    );
}

#[tokio::test]
async fn restored_history_is_replayed_before_the_prompt() {
    let transport = Arc::new(MockTransport::new(vec![MockBody::Events(vec![
        StreamEvent::Done {
            reason: StopReason::Stop,
        },
    ])]));
    let mut config = EngineConfig::new("primary");
    config.restored_messages = vec![
        ChatMessage {
            role: Role::User,
            content: "earlier question".into(),
            tool_calls: Vec::new(),
        },
        ChatMessage {
            role: Role::Assistant,
            content: "earlier answer".into(),
            tool_calls: Vec::new(),
        },
    ];
    let mut engine = EngineRuntime::start(
        config,
        resolver(vec![("primary", Arc::clone(&transport) as _)]),
    );
    engine
        .send(EngineCommand::SubmitPrompt {
            text: "follow up".into(),
        })
        .await
        .unwrap();
    let _ = collect_until_terminal(&mut engine).await;

    let requests = transport.requests();
    let messages = &requests[0].messages;
    assert_eq!(messages.len(), 3, "restored history plus the new prompt");
    assert_eq!(messages[0].role, Role::User);
    assert_eq!(messages[0].content, "earlier question");
    assert_eq!(messages[1].role, Role::Assistant);
    assert_eq!(messages[1].content, "earlier answer");
    assert_eq!(messages[2].role, Role::User);
    assert_eq!(messages[2].content, "follow up");
}

#[tokio::test]
async fn queued_prompts_each_get_a_well_formed_frame() {
    let workspace = workspace_with_hub_and_leaf();
    let transport = Arc::new(MockTransport::new(vec![
        MockBody::Events(vec![StreamEvent::Done {
            reason: StopReason::Stop,
        }]),
        MockBody::Events(vec![StreamEvent::Done {
            reason: StopReason::Stop,
        }]),
    ]));
    let mut config = EngineConfig::new("primary");
    config.genome_root = Some(workspace.path().to_path_buf());
    let mut engine = EngineRuntime::start(
        config,
        resolver(vec![("primary", Arc::clone(&transport) as _)]),
    );

    // Both prompts are in flight at once: the second queues behind the first,
    // so two refreshes run against one index.
    engine
        .send(EngineCommand::SubmitPrompt { text: "one".into() })
        .await
        .unwrap();
    engine
        .send(EngineCommand::SubmitPrompt { text: "two".into() })
        .await
        .unwrap();

    let mut finished = 0;
    while finished < 2 {
        match engine.recv().await {
            Some(EngineEvent::TurnFinished { .. }) | Some(EngineEvent::Failed { .. }) => {
                finished += 1
            }
            Some(_) => {}
            None => break,
        }
    }

    let requests = transport.requests();
    assert_eq!(requests.len(), 2, "both turns reached the provider");
    for request in &requests {
        assert_eq!(request.messages[0].role, Role::System);
        let frame = &request.messages[0].content;
        assert!(frame.starts_with("<genome>\n"), "{frame}");
        assert!(
            frame.ends_with("</genome>"),
            "frame must be closed: {frame}"
        );
    }
}

#[tokio::test]
async fn falls_back_after_transient_budget() {
    let primary = Arc::new(MockTransport::new(vec![
        MockBody::Err(TransportError::Retryable {
            status: Some(429),
            message: "limited".into(),
        }),
        MockBody::Err(TransportError::Retryable {
            status: Some(429),
            message: "limited".into(),
        }),
    ]));
    let backup = Arc::new(MockTransport::new(vec![MockBody::Events(vec![
        StreamEvent::TextDelta {
            id: BlockId::new("text"),
            text: "backup".into(),
        },
        StreamEvent::Done {
            reason: StopReason::Stop,
        },
    ])]));
    let mut config = EngineConfig::new("primary");
    config.fallback_models = vec!["backup".into()];
    config.max_transient_retries = 1;
    let mut engine = EngineRuntime::start(
        config,
        resolver(vec![
            ("primary", primary.clone()),
            ("backup", backup.clone()),
        ]),
    );

    engine
        .send(EngineCommand::SubmitPrompt { text: "hi".into() })
        .await
        .unwrap();
    let events = collect_until_terminal(&mut engine).await;

    assert_eq!(primary.call_count(), 2);
    assert_eq!(backup.call_count(), 1);
    assert!(events.iter().any(|event| matches!(event, EngineEvent::ModelSwitched { from, to, .. } if from == "primary" && to == "backup")));
}

#[tokio::test]
async fn does_not_retry_permanent_errors() {
    let primary = Arc::new(MockTransport::new(vec![MockBody::Err(
        TransportError::Fatal {
            status: Some(401),
            message: "unauthorized".into(),
        },
    )]));
    let backup = Arc::new(MockTransport::new(vec![MockBody::Events(vec![
        StreamEvent::Done {
            reason: StopReason::Stop,
        },
    ])]));
    let mut config = EngineConfig::new("primary");
    config.fallback_models = vec!["backup".into()];
    let mut engine = EngineRuntime::start(
        config,
        resolver(vec![
            ("primary", primary.clone()),
            ("backup", backup.clone()),
        ]),
    );

    engine
        .send(EngineCommand::SubmitPrompt { text: "hi".into() })
        .await
        .unwrap();
    let events = collect_until_terminal(&mut engine).await;

    assert_eq!(primary.call_count(), 1);
    assert_eq!(backup.call_count(), 0);
    assert!(matches!(
        events.last(),
        Some(EngineEvent::Failed {
            reason: titi_providers::ErrorReason::Rejected,
            ..
        })
    ));
}

#[tokio::test]
async fn cancel_aborts_an_in_flight_turn() {
    let transport = Arc::new(MockTransport::new(vec![MockBody::Events(vec![
        StreamEvent::Start,
        StreamEvent::TextDelta {
            id: BlockId::new("text"),
            text: "partial".into(),
        },
        StreamEvent::Done {
            reason: StopReason::Stop,
        },
    ])]));
    let mut engine = EngineRuntime::start(
        EngineConfig::new("primary"),
        resolver(vec![("primary", transport)]),
    );
    engine
        .send(EngineCommand::SubmitPrompt { text: "hi".into() })
        .await
        .unwrap();
    engine.send(EngineCommand::Cancel).await.unwrap();
    let events = collect_until_terminal(&mut engine).await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, EngineEvent::Cancelled { .. })),
        "{events:?}"
    );
}

#[tokio::test]
async fn follow_up_runs_after_active_turn() {
    let transport = Arc::new(MockTransport::new(vec![
        MockBody::Events(vec![
            StreamEvent::TextDelta {
                id: BlockId::new("one"),
                text: "first".into(),
            },
            StreamEvent::Done {
                reason: StopReason::Stop,
            },
        ]),
        MockBody::Events(vec![
            StreamEvent::TextDelta {
                id: BlockId::new("two"),
                text: "second".into(),
            },
            StreamEvent::Done {
                reason: StopReason::Stop,
            },
        ]),
    ]));
    let mut engine = EngineRuntime::start(
        EngineConfig::new("primary"),
        resolver(vec![("primary", transport)]),
    );
    engine
        .send(EngineCommand::SubmitPrompt { text: "one".into() })
        .await
        .unwrap();
    engine
        .send(EngineCommand::FollowUp { text: "two".into() })
        .await
        .unwrap();

    let first = collect_until_terminal(&mut engine).await;
    let second = collect_until_terminal(&mut engine).await;
    assert!(
        first
            .iter()
            .any(|event| matches!(event, EngineEvent::StreamDelta { text, .. } if text == "first"))
    );
    assert!(
        second.iter().any(
            |event| matches!(event, EngineEvent::StreamDelta { text, .. } if text == "second")
        )
    );
}
