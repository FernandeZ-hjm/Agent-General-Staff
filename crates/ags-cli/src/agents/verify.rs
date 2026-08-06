/// `ags agents verify` — composite host verification. Capability visibility
/// and the host-native project-memory lifecycle are independent requirements;
/// both are shown so an inherited MCP/index source cannot masquerade as a live
/// host closure.
pub(in crate::agents) fn cmd_agents_verify(host: &str, strict: bool, format: &str) {
    use ags_capability_governance::skill_body::console;

    let root = crate::context::capability_authority_root_or_exit("ags agents verify");
    let ctx = console::ConsoleContext::system(root);
    let capability = console::verify_host(&ctx, host);
    let target = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let agent = match ags_workspace_facts::AgentType::from_str(host) {
        Ok(agent) => agent,
        Err(error) => {
            eprintln!("ags agents verify: {error}");
            std::process::exit(2);
        }
    };
    let memory = ags_workspace_facts::compute_memory_lifecycle_for_host(&target, &agent);
    let native_probe_required =
        ags_host_integration::platform_spec(host).is_some_and(|spec| spec.mcp_probe.is_some());
    let native_probe = native_probe_required.then(|| ags_host_integration::inspect_host_mcp(host));
    let native_probe_ready = native_probe.as_ref().is_none_or(|report| {
        report.status == ags_host_integration::HostProbeStatus::Ready
            && report
                .find("ags")
                .is_some_and(|registration| registration.active)
    });
    let ok = capability.status == "ok" && memory.status == "full" && native_probe_ready;

    let output = serde_json::json!({
        "schema_version": "0.3.6-agent-host-verification",
        "host": host,
        "status": if ok { "ok" } else { "drifted" },
        "capability_visibility": capability,
        "memory_lifecycle": memory,
        "host_native_mcp": native_probe,
        "strict_ready": ok,
    });
    crate::output::emit(format, &output, || {
        format!(
            "{}\n\nNative memory lifecycle\n  host: {}\n  adapter: {}\n  status: {}\n  {}\n\nHost-native MCP probe\n  {}",
            console::render_verify_text(&capability),
            memory.host,
            memory.adapter,
            memory.status,
            memory.summary,
            native_probe.as_ref().map_or_else(
                || "not applicable for this host".to_string(),
                |report| format!("{:?}: {}", report.status, report.evidence),
            )
        )
    });
    if strict && !ok {
        std::process::exit(1);
    }
}
