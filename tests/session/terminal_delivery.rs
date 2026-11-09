use super::*;
use std::{
    future::Future,
    task::{Context, Poll, Waker},
    time::Duration,
};

#[tokio::test]
async fn waiting_operations_do_not_call_runtime_after_display_closes() {
    let mut command = tokio::process::Command::new("python3");
    command.args([
        "-u",
        "-c",
        r#"
import sys,json
print(json.dumps(dict(version=1,type='ready')),flush=True)
calls=0
for line in sys.stdin:
    request=json.loads(line)
    reply=dict(version=1,type='response',id=request['id'])
    if request['method']=='fixture.request_count':
        reply['result']=dict(calls=calls)
    else:
        calls+=1
        reply['error']=dict(code='unexpected_rpc',message='closed display touched Runtime')
    print(json.dumps(reply),flush=True)
"#,
    ]);
    let process = RuntimeProcess::start(command).await.unwrap();
    let mut errors = Vec::new();
    for operation in [
        SessionCommand::Send {
            session_id: "s".into(),
            message: "must not dispatch".into(),
        },
        SessionCommand::Create,
    ] {
        let delivery = Delivery::default();
        let held = delivery.lock().await;
        let (output, receiver) = super::super::channel::channel();
        let mut operation = Box::pin(execute(
            &process.client,
            operation,
            Some(1),
            &output,
            &delivery,
        ));
        let mut context = Context::from_waker(Waker::noop());
        assert!(
            matches!(operation.as_mut().poll(&mut context), Poll::Pending),
            "execute must pass its entry check and wait for the held delivery lock"
        );
        drop(receiver);
        assert!(output.is_closed());
        drop(held);
        errors.push(
            tokio::time::timeout(Duration::from_secs(2), operation)
                .await
                .unwrap()
                .unwrap_err()
                .code,
        );
    }
    let observed: Value = process
        .client
        .call("fixture.request_count", json!({}))
        .await
        .unwrap();
    process.finish().await.unwrap();
    assert_eq!(
        observed["calls"], 0,
        "an operation waiting for delivery lock dispatched after display closure"
    );
    assert_eq!(errors, ["display_closed", "display_closed"]);
}
