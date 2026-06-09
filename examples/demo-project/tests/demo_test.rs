#[test]
fn test_demo_project_basics() {
    assert_eq!(2 + 2, 4);
    assert_ne!(2 + 2, 5);
}

#[test]
fn test_demo_project_string() {
    let s = String::from("AGS");
    assert_eq!(s.len(), 3);
}
