#[path = "support/log_capture.rs"]
mod log_capture;

#[test]
#[should_panic(expected = "Isolated log-capture test did not execute successfully")]
fn empty_child_selection_is_not_reported_as_success() {
    log_capture::run_in_child("missing_log_capture_fixture");
}

#[test]
fn failing_child_is_not_reported_as_success() {
    const NAME: &str = "failing_child_is_not_reported_as_success";
    if std::env::var("TREAZURY_LOG_CAPTURE_CHILD").as_deref() == Ok(NAME) {
        panic!("intentional log-capture child failure");
    }
    let failure = std::panic::catch_unwind(|| log_capture::run_in_child(NAME))
        .expect_err("a failing child must fail its parent fixture");
    let message = failure.downcast_ref::<String>().unwrap();
    assert!(message.contains("Isolated log-capture test did not execute successfully"));
    assert!(message.contains("intentional log-capture child failure"));
}
