//! End-to-end pipeline tests (loader → executor).

#[test]
#[ignore = "executor not yet implemented for real modules"]
fn load_and_execute_exit_module() {
    // Load a minimal .dis module that contains a single `exit` instruction,
    // then execute it and verify clean exit.
    todo!()
}

#[test]
#[ignore = "executor not yet implemented for real modules"]
fn load_and_execute_hello_world() {
    // Load a Limbo-compiled hello world .dis module,
    // execute it, and verify stdout output.
    todo!()
}
