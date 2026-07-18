use request_router::{
    route_request, CliCapabilityId, DecisionStatus, EngineeringDemand, RequestContext,
    RequiredInput, RouteTarget, SkillDemand, TypedCliInput, REQUEST_DECISION_SCHEMA_VERSION,
};

fn route(request: &str) -> request_router::RequestDecision {
    route_request(RequestContext {
        request,
        handoff_contract: None,
    })
}

#[test]
fn bounded_content_transform_is_direct_response_only() {
    let decision = route("按已确认结构压缩这段内容，生成一张决策卡");

    assert_eq!(decision.schema_version, REQUEST_DECISION_SCHEMA_VERSION);
    assert_eq!(decision.status, DecisionStatus::Ready);
    assert_eq!(decision.targets, vec![RouteTarget::DirectResponse]);
}

#[test]
fn approved_json_field_normalization_does_not_route_to_brainstorming() {
    let decision = route("统一已批准 JSON 的字段名，不改语义");

    assert_eq!(decision.targets, vec![RouteTarget::DirectResponse]);
}

#[test]
fn new_cross_module_architecture_routes_to_system_architecture() {
    let decision = route("设计一个跨 MCP、CLI、Vault 的新系统架构和架构边界");

    assert_eq!(
        decision.targets,
        vec![RouteTarget::Skill {
            demand: SkillDemand::Engineering(EngineeringDemand::SystemArchitecture),
        }]
    );
}

#[test]
fn task_card_handoff_routes_to_one_business_cli_capability() {
    let decision = route_request(RequestContext {
        request: "按已确认方案生成任务卡交给 Claude Code",
        handoff_contract: Some("任务：实现已确认方案\n目标：完成并验证"),
    });

    assert_eq!(decision.status, DecisionStatus::Ready);
    assert_eq!(decision.targets.len(), 1);
    assert!(matches!(
        &decision.targets[0],
        RouteTarget::MachineCli {
            capability: CliCapabilityId::TaskCompile,
            input: TypedCliInput::ConfirmedHandoffContract { content },
        }
        if content == "任务：实现已确认方案\n目标：完成并验证"
    ));
}

#[test]
fn natural_task_card_delivery_phrases_route_to_compile() {
    for request in [
        "给我任务卡",
        "给我一张任务卡",
        "给我你执行的任务卡吧",
        "请提供这个方案的任务卡",
    ] {
        let decision = route_request(RequestContext {
            request,
            handoff_contract: Some("任务：实现已确认方案\n目标：完成并验证"),
        });

        assert_eq!(decision.status, DecisionStatus::Ready, "request: {request}");
        assert!(
            matches!(
                &decision.targets[..],
                [RouteTarget::MachineCli {
                    capability: CliCapabilityId::TaskCompile,
                    ..
                }]
            ),
            "request should route only to TaskCompile: {request}; got {:?}",
            decision.targets
        );
    }
}

#[test]
fn task_card_discussion_and_validation_do_not_route_to_compile() {
    for request in ["解释任务卡协议", "检查任务卡格式", "验证任务卡"] {
        let decision = route(request);
        assert!(
            !decision.targets.iter().any(|target| matches!(
                target,
                RouteTarget::MachineCli {
                    capability: CliCapabilityId::TaskCompile,
                    ..
                }
            )),
            "request must not compile a task card: {request}"
        );
    }
}

#[test]
fn canonical_task_card_routes_to_task_execute() {
    let decision = route("## 任务卡\n- Executor: Codex\n");

    assert!(matches!(
        &decision.targets[0],
        RouteTarget::MachineCli {
            capability: CliCapabilityId::TaskExecute,
            input: TypedCliInput::TaskCard { .. },
        }
    ));
}

#[test]
fn task_validation_requires_the_task_card_payload() {
    let decision = route("验证任务卡");

    assert_eq!(
        decision.status,
        DecisionStatus::InsufficientContext {
            missing: vec![RequiredInput::TaskCard],
        }
    );
    assert!(decision.targets.is_empty());
}

#[test]
fn task_validation_carries_the_structured_task_card_payload() {
    let decision = route("请验证任务卡\n## 任务卡\n- Executor: Codex\n");

    assert_eq!(decision.status, DecisionStatus::Ready);
    assert!(matches!(
        &decision.targets[0],
        RouteTarget::MachineCli {
            capability: CliCapabilityId::TaskValidate,
            input: TypedCliInput::TaskCard { content },
        } if content.starts_with("## 任务卡")
    ));
}

#[test]
fn receipt_validation_requires_the_receipt_payload() {
    let decision = route("验证回执");

    assert_eq!(
        decision.status,
        DecisionStatus::InsufficientContext {
            missing: vec![RequiredInput::Receipt],
        }
    );
    assert!(decision.targets.is_empty());
}

#[test]
fn task_card_request_without_confirmed_contract_is_insufficient() {
    let decision = route("生成任务卡交给 Claude Code");

    assert_eq!(
        decision.status,
        DecisionStatus::InsufficientContext {
            missing: vec![RequiredInput::ConfirmedHandoffContract],
        }
    );
    assert!(decision.targets.is_empty());
}

#[test]
fn skill_and_machine_cli_are_peer_targets() {
    let decision = route("诊断这个测试失败，并运行项目验证");

    assert_eq!(decision.targets.len(), 2);
    assert_eq!(
        decision.targets[0],
        RouteTarget::Skill {
            demand: SkillDemand::Engineering(EngineeringDemand::Debugging),
        }
    );
    assert!(matches!(
        decision.targets[1],
        RouteTarget::MachineCli {
            capability: CliCapabilityId::ProjectVerify,
            ..
        }
    ));
}

#[test]
fn ambiguous_generic_content_request_is_insufficient() {
    let decision = route("写一篇文章");

    assert_eq!(
        decision.status,
        DecisionStatus::InsufficientContext {
            missing: vec![RequiredInput::ContentKind],
        }
    );
    assert!(decision.targets.is_empty());
}

#[test]
fn direct_response_is_exclusive() {
    for request in [
        "压缩已确认内容并运行验证",
        "按确定结构改写，同时调用 debugging 技能",
    ] {
        let decision = route(request);
        if decision.targets.contains(&RouteTarget::DirectResponse) {
            assert_eq!(decision.targets, vec![RouteTarget::DirectResponse]);
        }
    }
}

#[test]
fn serialized_contract_uses_0_2_8_schema_and_tagged_targets() {
    let value = serde_json::to_value(route("设计跨模块的新系统架构")).unwrap();

    assert_eq!(value["schema_version"], "0.2.8-request-decision");
    assert_eq!(value["status"]["kind"], "ready");
    assert_eq!(value["targets"][0]["kind"], "skill");
}

#[test]
fn closed_skill_demand_catalog_has_no_duplicates() {
    let all = SkillDemand::all();
    let unique: std::collections::HashSet<_> = all.iter().copied().collect();

    assert_eq!(all.len(), unique.len());
    assert!(all.contains(&SkillDemand::Engineering(
        EngineeringDemand::SystemArchitecture
    )));
}
