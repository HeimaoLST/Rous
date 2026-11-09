#[tokio::test]
#[ignore = "requires explicit external SAKI_TEST_RUNTIME_BIN; covered by client::tests::real_runtime_contract"]
async fn plain_startup_creates_fresh_sessions_and_explicit_ids_still_restore() {
    crate::client::tests::startup_session_policy().await;
}
#[tokio::test]
#[ignore = "process child for public-protocol startup policy"]
async fn startup_session_child() {
    crate::app::run().await.expect("startup policy child");
}
