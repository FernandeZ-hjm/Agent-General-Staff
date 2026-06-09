/// Demo project — a minimal CLI that echoes input.
///
/// This is a synthetic project for demonstrating AGS task-card validation,
/// policy resolution, and verification workflows.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        println!("Demo project received: {}", args[1]);
    } else {
        println!("Demo project — AGS integration example");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn demo_test() {
        assert_eq!(2 + 2, 4);
    }
}
