use super::*;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
};
pub fn metadata(id: &str, title: &str) -> SessionMetadata {
    serde_json::from_value(json!({"schema_version":1,"id":id,"title":title,"title_source":"manual","created_at":0,"working_directory":"/tmp","model":"test-model","api":"openai-responses","compaction":{}})).unwrap()
}
struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let root = std::env::temp_dir().join(history::new_id().unwrap());
        std::fs::create_dir_all(root.join("work")).unwrap();
        Self(root.canonicalize().unwrap())
    }
    fn command(&self, binary: &Path, url: &str) -> Command {
        let mut command = Command::new(binary);
        command
            .current_dir(self.0.join("work"))
            .env_remove("SAKI_RUNTIME_CONFIG")
            .env("SAKI_SESSION_DIR", self.0.join("sessions"))
            .env("SAKI_API", "openai-responses")
            .env("SAKI_BASE_URL", url)
            .env("API_KEY", "local-fixture")
            .env("SAKI_MODEL", "fixture")
            .env("SAKI_CONTEXT_TOKENS", "65536");
        command
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Provider {
    url: String,
    calls: Arc<AtomicUsize>,
    hold: Arc<Notify>,
    release: Arc<Notify>,
    server: tokio::task::JoinHandle<()>,
}
impl Provider {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let hold = Arc::new(Notify::new());
        let arrived = hold.clone();
        let release = Arc::new(Notify::new());
        let released = release.clone();
        let server = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted=listener.accept()=>{
                        let (mut socket,_)=accepted.unwrap();let count=count.clone();let arrived=arrived.clone();let released=released.clone();
                        connections.spawn(async move{
                            let mut bytes=Vec::new();let mut one=[0;1];
                            while !bytes.ends_with(b"\r\n\r\n"){
                                if socket.read(&mut one).await.unwrap_or(0)==0{return;}
                                bytes.push(one[0]);
                            }
                            let header=String::from_utf8_lossy(&bytes);
                            let length=header.lines().find_map(|line|line.split_once(':').filter(|(name,_)|name.eq_ignore_ascii_case("content-length")).and_then(|(_,n)|n.trim().parse::<usize>().ok())).unwrap_or(0);
                            let mut body=vec![0;length];socket.read_exact(&mut body).await.unwrap();
                            count.fetch_add(1,Ordering::SeqCst);
                            if String::from_utf8_lossy(&body).contains("hold-me"){
                                arrived.notify_one();released.notified().await;
                            }
                            let request:Value=serde_json::from_slice(&body).unwrap();
                        let read_marker=String::from_utf8_lossy(&body).contains("read-marker") && !String::from_utf8_lossy(&body).contains("function_call_output");
                        let body=if read_marker {json!({"id":"fixture","status":"completed","output":[{"type":"function_call","status":"completed","id":"read-id","call_id":"read-marker-call","name":"read","arguments":"{\"path\":\"marker.txt\"}"}]})} else {json!({"id":"fixture","status":"completed","output":[{"type":"message","id":"answer","status":"completed","role":"assistant","content":[{"type":"output_text","text":"Fixture answer"}]}]})};
                        assert!(request.get("model").is_some());
                        let body=body.to_string();
                            let response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body);
                            let _=socket.write_all(response.as_bytes()).await;
                        });
                    }
                    _=connections.join_next(),if !connections.is_empty()=>{}
                }
            }
        });
        Self {
            url,
            calls,
            hold,
            release,
            server,
        }
    }
}
impl Drop for Provider {
    fn drop(&mut self) {
        self.server.abort();
    }
}
fn binary() -> PathBuf {
    std::env::var_os("SAKI_TEST_RUNTIME_BIN")
        .expect("set SAKI_TEST_RUNTIME_BIN to an independent Saki artifact")
        .into()
}
async fn next(events: &mut mpsc::UnboundedReceiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(15), events.recv())
        .await
        .expect("bounded Runtime event wait")
        .expect("Runtime event output")
}
async fn completed(process: &mut RuntimeProcess, task: &str) -> TaskFinished {
    loop {
        let event = next(&mut process.events).await;
        if event.task_id.as_deref() == Some(task) && event.event["type"] == "task_finished" {
            return serde_json::from_value(event.event).unwrap();
        }
    }
}
async fn stop(process: RuntimeProcess) {
    let _: Value = process
        .client
        .call("runtime.shutdown", json!({}))
        .await
        .unwrap();
    process.finish().await.unwrap();
}
#[tokio::test]
#[ignore = "requires an explicit independently built Saki artifact and uses local providers only"]
async fn real_runtime_contract() {
    tokio::time::timeout(Duration::from_secs(90),async {
        let provider=Provider::new().await;let root=Workspace::new();
        let mut process=RuntimeProcess::start(root.command(&binary(),&provider.url)).await.unwrap();
        let subscribed:Value=process.client.call("runtime.metadata.subscribe",json!({})).await.unwrap();assert_eq!(subscribed["subscribed"],true);
        let session:SessionState=process.client.call("session.create",json!({})).await.unwrap();
        let fresh:types::Snapshot=process.client.call("session.snapshot",json!({"session_id":session.session_id})).await.unwrap();
        assert_eq!(fresh.records.len(),1);assert!(fresh.context.context_tokens.unwrap()>0);
        assert_eq!(provider.calls.load(Ordering::SeqCst),0,"startup/snapshot must not request a model");
        let accepted:Value=process.client.call("task.send",json!({"session_id":session.session_id,"message":{"text":"first input"}})).await.unwrap();
        assert!(!accepted["task_id"].as_str().unwrap().is_empty());assert!(!accepted["message_id"].as_str().unwrap().is_empty());
        let mut metadata_seen=false;
        let finished:TaskFinished=loop {
            let event=next(&mut process.events).await;
            if event.event["type"]=="session_metadata_updated" {assert!(event.task_id.is_none());assert!(event.message_id.is_none());metadata_seen=true;}
            if event.task_id.as_deref()==accepted["task_id"].as_str() && event.event["type"]=="task_finished" {break serde_json::from_value(event.event).unwrap();}
        };
        assert_eq!(finished.status,"completed");
        let full:types::Snapshot=process.client.call("session.snapshot",json!({"session_id":session.session_id})).await.unwrap();
        assert!(full.records.iter().any(|r|matches!(&r.record,history::HistoryEntry::TaskFinished{task_id,status:history::TaskStatus::Completed,..} if task_id==accepted["task_id"].as_str().unwrap())));
        let before:types::Snapshot=process.client.call("session.snapshot",json!({"session_id":session.session_id,"before_task_id":accepted["task_id"]})).await.unwrap();
        assert_eq!(before.records.len(),1);
        let missing=process.client.call::<Value>("session.snapshot",json!({"session_id":session.session_id,"before_task_id":"missing-task"})).await.unwrap_err();assert_eq!(missing.code,"not_found");
        // Wait for the service-owned title invalidation, never generate a title in Rous.
        while !metadata_seen {let event=next(&mut process.events).await;if event.event["type"]=="session_metadata_updated"{assert!(event.task_id.is_none());assert!(event.message_id.is_none());metadata_seen=true;}}
        let listed:Value=process.client.call("session.list",json!({})).await.unwrap();
        assert_eq!(listed["sessions"][0]["metadata"]["title"],"Fixture answer");
        let accepted:Value=process.client.call("task.send",json!({"session_id":session.session_id,"message":{"text":"hold-me"}})).await.unwrap();
        tokio::time::timeout(Duration::from_secs(10),provider.hold.notified()).await.unwrap();
        let reject=process.client.call::<Value>("task.send",json!({"session_id":session.session_id,"message":{"text":"must not queue"}})).await.unwrap_err();assert_eq!(reject.code,"busy");
        let wrong:Value=process.client.call("task.cancel",json!({"session_id":session.session_id,"task_id":"wrong-task"})).await.unwrap();assert_eq!(wrong["status"],"not_running");
        let cancelled:Value=process.client.call("task.cancel",json!({"session_id":session.session_id,"task_id":accepted["task_id"]})).await.unwrap();assert_eq!(cancelled["status"],"requested");
        assert_eq!(completed(&mut process,accepted["task_id"].as_str().unwrap()).await.status,"interrupted");
        stop(process).await;
        let process=RuntimeProcess::start(root.command(&binary(),&provider.url)).await.unwrap();
        let _:SessionState=process.client.call("session.load",json!({"session_id":session.session_id})).await.unwrap();
        let restored:types::Snapshot=process.client.call("session.snapshot",json!({"session_id":session.session_id})).await.unwrap();
        assert_eq!(restored.session.status,"interrupted");
        stop(process).await;
        display_failure_stops_admission_and_drains_runtime().await;
        navigation_and_cancel().await;
        models_and_directories().await;
        other_api_is_rejected().await;
        startup_session_policy().await;
        println!("Rous independent stdio: fresh/resume/snapshot/metadata/title/send/busy/targeted-cancel/shutdown PASS; external requests=0");
    }).await.expect("contract fixture exceeded 90 seconds");
}
pub async fn startup_session_policy() {
    let provider = Provider::new().await;
    let root = Workspace::new();
    let mut process = RuntimeProcess::start(root.command(&binary(), &provider.url))
        .await
        .unwrap();
    let session: SessionState = process
        .client
        .call("session.create", json!({}))
        .await
        .unwrap();
    let task: Value = process
        .client
        .call(
            "task.send",
            json!({"session_id":session.session_id,"message":{"text":"seed-old-history-marker"}}),
        )
        .await
        .unwrap();
    assert_eq!(
        completed(&mut process, task["task_id"].as_str().unwrap())
            .await
            .status,
        "completed"
    );
    stop(process).await;
    let calls = provider.calls.load(Ordering::SeqCst);
    for (selected, mode) in [
        (None, "console"),
        (None, "console"),
        (Some("new"), "console"),
        (Some(session.session_id.as_str()), "console"),
        (Some("missing-session-id"), "console"),
        (None, "unknown-mode"),
    ] {
        let mut command = root.command(&std::env::current_exe().unwrap(), &provider.url);
        command
            .args([
                "--ignored",
                "--exact",
                "--nocapture",
                "frontend::console::tests::startup::startup_session_child",
            ])
            .env("SAKI_UI", mode)
            .env("SAKI_RUNTIME_BIN", binary())
            .env_remove("SAKI_SESSION")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        if let Some(id) = selected {
            command.env("SAKI_SESSION", id);
        }
        let output = tokio::time::timeout(Duration::from_secs(15), command.output())
            .await
            .unwrap()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            output.status.success(),
            selected != Some("missing-session-id"),
            "{text}"
        );
        if mode == "unknown-mode" {
            assert!(text.contains("using console"), "{text}");
        }
        if selected == Some(session.session_id.as_str()) {
            assert!(text.contains("seed-old-history-marker"), "{text}");
        } else {
            assert!(!text.contains("seed-old-history-marker"), "{text}");
        }
    }
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        calls,
        "fresh/resume startup requested a model"
    );
    let process = RuntimeProcess::start(root.command(&binary(), &provider.url))
        .await
        .unwrap();
    let sessions: Value = process
        .client
        .call("session.list", json!({}))
        .await
        .unwrap();
    assert_eq!(sessions["sessions"].as_array().unwrap().len(), 5);
    stop(process).await;
}

struct UiEvents {
    receiver: crate::session::SessionEventReceiver,
    pending: std::collections::VecDeque<crate::session::SessionEventPacket>,
}
impl UiEvents {
    async fn take(
        &mut self,
        predicate: impl Fn(&crate::session::SessionEventPacket) -> bool,
    ) -> crate::session::SessionEventPacket {
        if let Some(index) = self.pending.iter().position(&predicate) {
            return self.pending.remove(index).unwrap();
        }
        loop {
            let packet = tokio::time::timeout(Duration::from_secs(10), self.receiver.recv())
                .await
                .unwrap()
                .expect("UI event stream");
            if predicate(&packet) {
                return packet;
            }
            self.pending.push_back(packet);
        }
    }
    async fn reply(&mut self, id: u64) -> crate::session::SessionEventPacket {
        self.take(|packet| packet.request_id == Some(id)).await
    }
    async fn finished(&mut self, task: &str) -> TaskFinished {
        let packet = self
            .take(|packet| {
                packet
                    .association
                    .as_ref()
                    .is_some_and(|a| a.task_id == task)
                    && matches!(packet.event, crate::session::SessionEvent::Finished(_))
            })
            .await;
        match packet.event {
            crate::session::SessionEvent::Finished(finished) => finished,
            _ => unreachable!(),
        }
    }
}
async fn navigation_and_cancel() {
    use crate::session::{
        SessionCommand as C, SessionEvent as E, SessionManager, SessionRequest as R,
    };
    let root = Workspace::new();
    let provider = Provider::new().await;
    let process = RuntimeProcess::start(root.command(&binary(), &provider.url))
        .await
        .unwrap();
    let client = process.client.clone();
    let a: SessionState = client.call("session.create", json!({})).await.unwrap();
    let b: SessionState = client.call("session.create", json!({})).await.unwrap();
    let manager = SessionManager::new(process, Some(a.session_id.clone()));
    let (tx, requests) = mpsc::channel(32);
    let (output, receiver) = crate::session::channel::channel();
    let worker = tokio::spawn(manager.run(requests, output));
    let mut ui = UiEvents {
        receiver,
        pending: Default::default(),
    };
    ui.take(|p| matches!(p.event, E::Opened { .. })).await;
    tx.send(R {
        request_id: 1,
        command: C::Send {
            session_id: a.session_id.clone(),
            message: "first complete".into(),
        },
    })
    .await
    .unwrap();
    let first = ui.reply(1).await.association.unwrap();
    assert_eq!(ui.finished(&first.task_id).await.status, "completed");
    ui.take(
        |packet| matches!(&packet.event,E::TitleUpdated(metadata) if metadata.id==a.session_id),
    )
    .await;
    tx.send(R {
        request_id: 2,
        command: C::Send {
            session_id: a.session_id.clone(),
            message: "hold-me".into(),
        },
    })
    .await
    .unwrap();
    let second = ui.reply(2).await.association.unwrap();
    tokio::time::timeout(Duration::from_secs(10), provider.hold.notified())
        .await
        .unwrap();
    tx.send(R {
        request_id: 3,
        command: C::List,
    })
    .await
    .unwrap();
    assert!(matches!(ui.reply(3).await.event,E::Listed(items) if items.len()==2));
    tx.send(R {
        request_id: 4,
        command: C::Open {
            session_id: b.session_id.clone(),
        },
    })
    .await
    .unwrap();
    assert!(
        matches!(ui.reply(4).await.event,E::Opened{metadata,state,stream:None,..} if metadata.id==b.session_id && state.active_task.is_none())
    );
    tx.send(R {
        request_id: 5,
        command: C::Open {
            session_id: a.session_id.clone(),
        },
    })
    .await
    .unwrap();
    match ui.reply(5).await.event {
        E::Opened {
            metadata,
            replay,
            state,
            stream,
            ..
        } => {
            assert_eq!(metadata.id, a.session_id);
            assert_eq!(state.state, "running");
            assert_eq!(stream.unwrap(), second);
            assert_eq!(
                replay.status,
                history::TaskStatus::Completed,
                "prefix before active task must preserve the prior completed status"
            );
            assert!(
                !format!(
                    "{:?}",
                    crate::frontend::tui::state::App::restore(&replay).records
                )
                .contains("Previous task: Unknown")
            );
        }
        _ => panic!("reopen must return a real snapshot"),
    }
    let before = provider.calls.load(Ordering::SeqCst);
    tx.send(R {
        request_id: 6,
        command: C::Cancel {
            session_id: a.session_id.clone(),
            task_id: first.task_id,
        },
    })
    .await
    .unwrap();
    assert!(matches!(ui.reply(6).await.event,E::Cancelled{status} if status=="not_running"));
    let active: Value = client
        .call(
            "task.status",
            json!({"session_id":a.session_id,"task_id":second.task_id}),
        )
        .await
        .unwrap();
    assert_eq!(active["status"], "running");
    let b_snapshot: types::Snapshot = client
        .call("session.snapshot", json!({"session_id":b.session_id}))
        .await
        .unwrap();
    assert_eq!(b_snapshot.records.len(), 1, "viewing B must not dispatch B");
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        before,
        "view/list/cancel old task caused a model request"
    );
    tx.send(R {
        request_id: 7,
        command: C::Cancel {
            session_id: a.session_id.clone(),
            task_id: second.task_id.clone(),
        },
    })
    .await
    .unwrap();
    assert!(matches!(ui.reply(7).await.event,E::Cancelled{status} if status=="requested"));
    assert_eq!(ui.finished(&second.task_id).await.status, "interrupted");
    drop(tx);
    drop(ui);
    worker.await.unwrap().unwrap();
}
async fn models_and_directories() {
    let root = Workspace::new();
    let provider = Provider::new().await;
    let mut process = RuntimeProcess::start(root.command(&binary(), &provider.url))
        .await
        .unwrap();
    let mut ids = Vec::new();
    for (name, capacity) in [("a", 65536), ("b", 98304)] {
        let path = root.0.join(name);
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("marker.txt"), format!("marker-{name}")).unwrap();
        let session:SessionState=process.client.call("session.create",json!({"storage_schema":2,"config":{
            "connection_id":"default","model":format!("fixture-{name}"),"working_directory":path,"system_prompt":format!("system-{name}"),
            "compaction":{"context_budget_tokens":capacity,"reserve_tokens":8192,"keep_recent_tokens":8192,"max_summary_tokens":4096}
        }})).await.unwrap();
        let task: Value = process
            .client
            .call(
                "task.send",
                json!({"session_id":session.session_id,"message":{"text":"read-marker"}}),
            )
            .await
            .unwrap();
        let finished = completed(&mut process, task["task_id"].as_str().unwrap()).await;
        assert_eq!(finished.status, "completed", "{:?}", finished);
        let snapshot: types::Snapshot = process
            .client
            .call("session.snapshot", json!({"session_id":session.session_id}))
            .await
            .unwrap();
        assert_eq!(snapshot.metadata.model, format!("fixture-{name}"));
        assert_eq!(snapshot.metadata.working_directory, path);
        assert_eq!(snapshot.context.context_window, Some(capacity));
        assert!(snapshot.records.iter().any(|r|matches!(&r.record,history::HistoryEntry::Messages{items} if items.iter().any(|item|matches!(item,types::InputItem::ToolResult{output,..} if output.contains(&format!("marker-{name}")))))));
        ids.push(session.session_id);
    }
    let _: SessionState = process
        .client
        .call("session.load", json!({"session_id":ids[0]}))
        .await
        .unwrap();
    let again: types::Snapshot = process
        .client
        .call("session.snapshot", json!({"session_id":ids[0]}))
        .await
        .unwrap();
    assert_eq!(again.metadata.model, "fixture-a");
    assert_eq!(again.metadata.working_directory, root.0.join("a"));
    stop(process).await;
}
async fn other_api_is_rejected() {
    let root = Workspace::new();
    let provider = Provider::new().await;
    let mut command = root.command(&binary(), &provider.url);
    command.env("SAKI_API", "openai-chat-completions");
    let process = RuntimeProcess::start(command).await.unwrap();
    let session: SessionState = process
        .client
        .call("session.create", json!({}))
        .await
        .unwrap();
    let before: Value = process
        .client
        .call("session.history", json!({"session_id":session.session_id}))
        .await
        .unwrap();
    stop(process).await;
    let process = RuntimeProcess::start(root.command(&binary(), &provider.url))
        .await
        .unwrap();
    let error = process
        .client
        .call::<SessionState>("session.load", json!({"session_id":session.session_id}))
        .await
        .unwrap_err();
    assert_eq!(error.code, "api_mismatch");
    let after: Value = process
        .client
        .call("session.history", json!({"session_id":session.session_id}))
        .await
        .unwrap();
    assert_eq!(
        before["records"], after["records"],
        "incompatible open changed canonical history"
    );
    let list: Value = process
        .client
        .call("session.list", json!({}))
        .await
        .unwrap();
    assert_eq!(
        list["sessions"].as_array().unwrap().len(),
        1,
        "incompatible open created a fallback session"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    stop(process).await;
}

fn fake_child(script: &str) -> Command {
    let mut command = Command::new("python3");
    command.args(["-u", "-c", &format!("import sys,json,os,socket\ndef emit(v):\n print(json.dumps(dict(version=1,**v)),flush=True)\n{script}")]);
    command
}
fn short_timeouts() -> Timeouts {
    Timeouts {
        startup: Duration::from_secs(5),
        write: Duration::from_millis(100),
        response: Duration::from_millis(100),
    }
}
#[tokio::test]
async fn partial_write_closes_input_before_another_request() {
    let root = Workspace::new();
    let captured = root.0.join("input");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut command = fake_child(
        "emit(dict(type='ready'))\nfirst=sys.stdin.buffer.read(1)\ns=socket.create_connection(('127.0.0.1',int(os.environ['CONTROL_PORT'])))\ns.recv(1)\nrest=sys.stdin.buffer.read()\nopen(os.environ['CAPTURE'],'wb').write(first+rest)",
    );
    command
        .env(
            "CONTROL_PORT",
            listener.local_addr().unwrap().port().to_string(),
        )
        .env("CAPTURE", &captured);
    let process = RuntimeProcess::start_with_options(command, short_timeouts())
        .await
        .unwrap();
    let client = process.client.clone();
    let writing = tokio::spawn(async move {
        client
            .call::<Value>("large", json!({"text":"x".repeat(2*1024*1024)}))
            .await
    });
    let (mut control, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let first = writing.await.unwrap().unwrap_err();
    let second = process
        .client
        .call::<Value>("must-not-write-second-frame", json!({}))
        .await
        .unwrap_err();
    control.write_all(b"x").await.unwrap();
    process.finish().await.unwrap();
    assert_eq!(first.code, "delivery_unknown");
    assert_eq!(second.code, "disconnected");
    let bytes = std::fs::read(captured).unwrap();
    assert!(!bytes.is_empty() && bytes.len() < 2 * 1024 * 1024);
    assert!(
        !bytes.contains(&b'\n'),
        "a later request was appended to a partial frame"
    );
}
#[tokio::test]
async fn response_timeout_preserves_a_fully_written_connection() {
    let process=RuntimeProcess::start_with_options(fake_child("emit(dict(type='ready'))\nfor line in sys.stdin:\n r=json.loads(line)\n if r['method']!='slow': emit(dict(type='response',id=r['id'],result=dict(ok=True)))"),short_timeouts()).await.unwrap();
    let error = process
        .client
        .call::<Value>("slow", json!({}))
        .await
        .unwrap_err();
    let response: Value = process.client.call("control", json!({})).await.unwrap();
    process.finish().await.unwrap();
    assert_eq!(error.code, "delivery_unknown");
    assert_eq!(response["ok"], true);
}
#[tokio::test]
async fn small_event_burst_does_not_block_control_responses() {
    let mut process=RuntimeProcess::start(fake_child("emit(dict(type='ready'))\nfor line in sys.stdin:\n r=json.loads(line)\n if r['method']=='burst':\n  for n in range(129): emit(dict(type='event',session_id='s',task_id='t',message_id='m',event=dict(type='response_text_delta',item_id='answer',delta=str(n))))\n emit(dict(type='response',id=r['id'],result=dict(ok=True)))")).await.unwrap();
    let burst: Value = process.client.call("burst", json!({})).await.unwrap();
    let control: Value = process.client.call("control", json!({})).await.unwrap();
    assert_eq!(burst["ok"], true);
    assert_eq!(control["ok"], true);
    for n in 0..129 {
        let event = next(&mut process.events).await;
        assert_eq!(event.event["delta"], n.to_string());
    }
    process.finish().await.unwrap();
}
#[tokio::test]
async fn event_byte_budget_disconnects_without_rpc_deadlock() {
    let process=RuntimeProcess::start(fake_child("emit(dict(type='ready'))\nfor line in sys.stdin:\n r=json.loads(line)\n for n in range(9): emit(dict(type='event',session_id='s',event=dict(type='response_text_delta',item_id='answer',delta='x'*1048576)))\n emit(dict(type='response',id=r['id'],result=dict(ok=True)))")).await.unwrap();
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        process.client.call::<Value>("overflow", json!({})),
    )
    .await
    .unwrap();
    assert_eq!(response.unwrap_err().code, "disconnected");
    assert!(process.finish().await.is_err());
}
#[tokio::test]
async fn ready_budget_allows_catalog_fallback_before_ready() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut command = fake_child(
        "s=socket.create_connection(('127.0.0.1',int(os.environ['CONTROL_PORT'])))\ns.recv(1)\nemit(dict(type='ready'))\nsys.stdin.buffer.read()",
    );
    command.env(
        "CONTROL_PORT",
        listener.local_addr().unwrap().port().to_string(),
    );
    let startup = tokio::spawn(RuntimeProcess::start(command));
    let (mut control, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
        .await
        .unwrap()
        .unwrap();
    // The child is alive before advancing beyond the 20-second catalog fallback.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(21)).await;
    tokio::time::resume();
    let released = control.write_all(b"x").await;
    let result = startup.await.unwrap();
    if let Ok(process) = result {
        process.finish().await.unwrap();
        released.unwrap();
    } else {
        panic!(
            "ready after normal catalog fallback was rejected: {}",
            result.err().unwrap()
        );
    }
}
#[tokio::test]
async fn startup_failure_reaps_child_and_readers() {
    for invalid_frame in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut command = fake_child(
            "s=socket.create_connection(('127.0.0.1',int(os.environ['CONTROL_PORT'])))\ns.sendall((str(os.getpid())+'\\n').encode())\nif os.environ['INVALID']=='true': emit(dict(type='invalid'))\ns.recv(1)",
        );
        command
            .env(
                "CONTROL_PORT",
                listener.local_addr().unwrap().port().to_string(),
            )
            .env("INVALID", invalid_frame.to_string());
        let timeouts = Timeouts {
            startup: Duration::from_secs(30),
            ..short_timeouts()
        };
        let startup = tokio::spawn(RuntimeProcess::start_with_options(command, timeouts));
        let (mut control, _) = tokio::time::timeout(Duration::from_secs(10), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let pid = tokio::time::timeout(Duration::from_secs(10), async {
            let mut pid = Vec::new();
            let mut one = [0];
            while !pid.ends_with(b"\n") {
                control.read_exact(&mut one).await.unwrap();
                pid.push(one[0]);
            }
            pid
        })
        .await
        .unwrap();
        let pid = String::from_utf8(pid)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap();
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(31)).await;
        tokio::time::resume();
        assert!(
            tokio::time::timeout(Duration::from_secs(5), startup)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "startup returned before child {pid} was reaped"
        );
    }
}

async fn display_failure_stops_admission_and_drains_runtime() {
    use crate::session::{
        SessionCommand as C, SessionEvent as E, SessionManager, SessionRequest as R,
    };
    let root = Workspace::new();
    let provider = Provider::new().await;
    let process = RuntimeProcess::start(root.command(&binary(), &provider.url))
        .await
        .unwrap();
    let session: SessionState = process
        .client
        .call("session.create", json!({}))
        .await
        .unwrap();
    let _: Value = process
        .client
        .call(
            "session.rename",
            json!({"session_id":session.session_id,"title":"Manual"}),
        )
        .await
        .unwrap();
    let manager = SessionManager::new(process, Some(session.session_id.clone()));
    let (requests, input) = mpsc::channel(32);
    let (output, receiver) = crate::session::channel::channel();
    let display = output.clone();
    let worker = tokio::spawn(manager.run(input, output));
    let mut ui = UiEvents {
        receiver,
        pending: Default::default(),
    };
    ui.take(|packet| matches!(packet.event, E::Opened { .. }))
        .await;
    requests
        .send(R {
            request_id: 1,
            command: C::Send {
                session_id: session.session_id.clone(),
                message: "hold-me".into(),
            },
        })
        .await
        .unwrap();
    assert!(matches!(ui.reply(1).await.event, E::Accepted));
    ui.take(|packet| {
        matches!(
            packet.event,
            E::Agent {
                event: AgentEvent::ModelRequestStarted { .. },
                ..
            }
        )
    })
    .await;
    tokio::time::timeout(Duration::from_secs(5), provider.hold.notified())
        .await
        .unwrap();
    for _ in 0..32 {
        display.send(E::Listed(vec![])).await.unwrap();
    }
    assert!(
        display
            .try_send_reply(
                2,
                E::Cancelled {
                    status: "requested".into()
                }
            )
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(5), requests.closed())
        .await
        .expect("display failure did not stop manager admission");
    assert!(
        requests
            .try_send(R {
                request_id: 3,
                command: C::Create
            })
            .is_err()
    );
    assert!(
        !worker.is_finished(),
        "display loss must drain the accepted task, not cancel it"
    );
    provider.release.notify_one();
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(display.is_closed());
    assert!(ui.receiver.recv().await.is_none());
    let process = RuntimeProcess::start(root.command(&binary(), &provider.url))
        .await
        .unwrap();
    let _: SessionState = process
        .client
        .call("session.load", json!({"session_id":session.session_id}))
        .await
        .unwrap();
    let snapshot: types::Snapshot = process
        .client
        .call("session.snapshot", json!({"session_id":session.session_id}))
        .await
        .unwrap();
    assert_eq!(snapshot.session.status, "completed");
    assert!(snapshot.records.iter().any(|record| matches!(
        &record.record,
        history::HistoryEntry::TaskFinished {
            status: history::TaskStatus::Completed,
            ..
        }
    )));
    let sessions: Value = process
        .client
        .call("session.list", json!({}))
        .await
        .unwrap();
    assert_eq!(
        sessions["sessions"].as_array().unwrap().len(),
        1,
        "closed manager admitted a new session"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    stop(process).await;
}
