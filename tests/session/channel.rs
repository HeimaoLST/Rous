use super::*;
use crate::client::TaskFinished;

#[tokio::test]
async fn terminal_enqueue_timeout_closes_display_and_releases_packets() {
    let (sender, mut receiver) = channel();
    for _ in 0..32 {
        sender.send(SessionEvent::Accepted).await.unwrap();
    }
    let mut terminal = Box::pin(sender.send(SessionEvent::Finished(TaskFinished {
        status: "completed".into(),
        error: None,
        persistence: None,
        history_required: false,
    })));
    tokio::time::pause();
    tokio::select! {biased;
        result=&mut terminal=>panic!("full display unexpectedly accepted terminal: {result:?}"),
        _=tokio::task::yield_now()=>{}
    }
    tokio::time::advance(Duration::from_secs(31)).await;
    tokio::time::resume();
    assert!(terminal.await.is_err());
    assert!(
        sender.is_closed(),
        "lost terminal must close display even while receiver is alive"
    );
    tokio::time::timeout(Duration::from_secs(1), sender.closed())
        .await
        .unwrap();
    assert!(sender.send(SessionEvent::Accepted).await.is_err());
    assert!(
        receiver.recv().await.is_none(),
        "partial queued display must not resume as healthy"
    );
    assert_eq!(sender.bytes.available_permits(), 8 * 1024 * 1024);
}
#[tokio::test]
async fn oversized_projection_and_full_control_reply_close_display() {
    let (sender, mut receiver) = channel();
    let oversized = SessionEvent::Error {
        operation: "test",
        error: crate::client::RuntimeError::new("test", "x".repeat(8 * 1024 * 1024)),
    };
    assert!(sender.send(oversized).await.is_err());
    assert!(sender.is_closed());
    assert!(receiver.recv().await.is_none());
    let (sender, mut receiver) = channel();
    for _ in 0..32 {
        sender.send(SessionEvent::Accepted).await.unwrap();
    }
    assert!(
        sender
            .try_send_reply(
                10,
                SessionEvent::Cancelled {
                    status: "requested".into()
                }
            )
            .is_err()
    );
    assert!(
        sender.is_closed(),
        "a critical reply cannot disappear while admission continues"
    );
    assert!(sender.try_send_reply(11, SessionEvent::Accepted).is_err());
    assert!(receiver.recv().await.is_none());
}
