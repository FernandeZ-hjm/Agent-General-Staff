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
    let ok = capability.status == "ok" && memory.status == "full";

    match format {
        "json" => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": "0.3.0-agent-host-verification",
                "host": host,
                "status": if ok { "ok" } else { "drifted" },
                "capability_visibility": capability,
                "memory_lifecycle": memory,
                "strict_ready": ok,
            }))
            .unwrap_or_default()
        ),
        _ => {
            println!("{}", console::render_verify_text(&capability));
            println!();
            println!("Native memory lifecycle");
            println!("  host: {}", memory.host);
            println!("  adapter: {}", memory.adapter);
            println!("  status: {}", memory.status);
            println!("  {}", memory.summary);
        }
    }
    if strict && !ok {
        std::process::exit(1);
    }
}
