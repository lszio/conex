use conex_sdk::{
    Client, ConnectOptions, Contract, Execution, Method, ModuleRef, State, state::StateEvent,
};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::net::TcpListener;
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        Message,
        handshake::server::{Callback, ErrorResponse, Request, Response},
    },
};

type Ws = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

struct JsonProfile;
impl Callback for JsonProfile {
    fn on_request(self, _: &Request, mut response: Response) -> Result<Response, ErrorResponse> {
        response.headers_mut().insert(
            "sec-websocket-protocol",
            "conex-jsonrpc2-wss".parse().unwrap(),
        );
        Ok(response)
    }
}

const REG: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
fn contract() -> Arc<Contract> {
    Arc::new(
        Contract::new(
            "conex.counter",
            BTreeMap::from([(
                "counter/add".into(),
                Method::new(
                    "conex.CounterAddInput",
                    "conex.CounterResult",
                    Execution::NonReplayable,
                ),
            )]),
            BTreeMap::new(),
        )
        .unwrap(),
    )
}
fn state_contract() -> Arc<Contract> {
    Arc::new(
        Contract::new(
            "conex.counter",
            BTreeMap::from([(
                "counter/add".into(),
                Method::new(
                    "conex.CounterAddInput",
                    "conex.CounterResult",
                    Execution::NonReplayable,
                ),
            )]),
            BTreeMap::from([(
                "value".into(),
                State::new("conex.CounterAddInput", "conex.CounterAddInput"),
            )]),
        )
        .unwrap(),
    )
}
fn target() -> ModuleRef {
    ModuleRef {
        space_id: "s".into(),
        provider_endpoint_id: "counter".into(),
        registration_id: REG.into(),
        contract_id: "conex.counter".into(),
    }
}
/// Bind, accept, and complete the conex/hello + conex/ready exchange. The spawned
/// task resolves to the negotiated server-side ws once the client has connected.
async fn ws_peer() -> (String, tokio::task::JoinHandle<Ws>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/", listener.local_addr().unwrap());
    let limits = json!({"maxFrameBytes":262144,"maxInflight":8,"maxQueuedBytes":1048576,"timeoutMs":2000,"maxSubscriptions":8,"maxModules":8});
    let peer = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = accept_hdr_async(tcp, JsonProfile).await.unwrap();
        let hello: Value =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        let methods = hello["params"]["input"]["requires"].clone();
        send_text(&mut ws, json!({"jsonrpc":"2.0","id":hello["id"],"result":{"bindingId":REG,"profileId":"conex-jsonrpc2-wss","plane":"PLANE_BROKER","provides":methods,"limits":limits.clone()}}).to_string()).await;
        let ready: Value =
            serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        send_text(&mut ws, json!({"jsonrpc":"2.0","id":ready["id"],"result":{"negotiationId":REG,"profileId":"conex-jsonrpc2-wss","plane":"PLANE_BROKER","provides":methods,"limits":limits,"linkIdentity":{"linkId":REG,"peerId":"hub","tenantId":"t"}}}).to_string()).await;
        ws
    });
    (url, peer)
}
async fn send_text(ws: &mut Ws, text: String) {
    ws.send(Message::text(text)).await.unwrap();
}
async fn recv_frame(ws: &mut Ws) -> Value {
    serde_json::from_str(ws.next().await.unwrap().unwrap().to_text().unwrap()).unwrap()
}
fn snapshot_result(request_id: &Value, subscription_id: &str) -> String {
    json!({"jsonrpc":"2.0","id":request_id,"result":{"subscriptionId":subscription_id,"registrationId":REG,"snapshot":{"revision":"9007199254740993","value":{"delta":0},"sampledAtMs":"0","quality":"current"}}}).to_string()
}
fn update_notice(context: &Value, id: &str, seq: u8, revision: u64, delta: i64) -> String {
    json!({"jsonrpc":"2.0","method":"state/update","params":{"context":context,"input":{"subscriptionId":id,"seq":seq.to_string(),"sample":{"revision":revision.to_string(),"value":{"delta":delta},"sampledAtMs":"0","quality":"current"}}}}).to_string()
}
fn end_notice(context: &Value, id: &str) -> String {
    json!({"jsonrpc":"2.0","method":"state/end","params":{"context":context,"input":{"subscriptionId":id,"error":{"code":"ERROR_CODE_UNAVAILABLE","message":"gone","diagnosticId":"","execution":"completed","retry":"safe"}}}}).to_string()
}
async fn server(
    disconnect: bool,
    count: Arc<AtomicUsize>,
) -> (
    String,
    tokio::task::JoinHandle<()>,
    tokio::sync::oneshot::Receiver<()>,
) {
    let (url, peer) = ws_peer().await;
    let (committed, commit) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut ws = peer.await.unwrap();
        let invoke = recv_frame(&mut ws).await;
        assert_eq!(invoke["method"], "module/invoke");
        count.fetch_add(
            invoke["params"]["input"]["argument"]["delta"]
                .as_u64()
                .unwrap() as usize,
            Ordering::SeqCst,
        );
        let _ = committed.send(());
        if disconnect {
            ws.close(None).await.unwrap();
            return;
        }
        let cancel = recv_frame(&mut ws).await;
        assert_eq!(cancel["method"], "module/cancel");
        send_text(
            &mut ws,
            json!({"jsonrpc":"2.0","id":cancel["id"],"result":{"requested":true}}).to_string(),
        )
        .await;
        send_text(&mut ws, json!({"jsonrpc":"2.0","id":invoke["id"],"result":{"count":count.load(Ordering::SeqCst),"revision":"1"}}).to_string()).await;
        while let Some(Ok(msg)) = ws.next().await {
            if msg.is_close() {
                break;
            }
        }
    });
    (url, task, commit)
}
async fn connect(url: String) -> Client {
    let mut options = ConnectOptions::new(url, "dev-token");
    options.allow_loopback_ws = true;
    Client::connect(options).await.unwrap()
}
/// One connection hosting two subscriptions. The test drives each step via
/// `step`; the fixture acks a batch over `sent` only after its frames are sent,
/// so every assertion observes frames the client has already processed.
async fn subscribe_peer() -> (
    String,
    tokio::task::JoinHandle<()>,
    tokio::sync::mpsc::UnboundedSender<u8>,
    tokio::sync::mpsc::UnboundedReceiver<u8>,
) {
    let (url, peer) = ws_peer().await;
    let (step, mut cmd) = tokio::sync::mpsc::unbounded_channel::<u8>();
    let (sent_tx, sent) = tokio::sync::mpsc::unbounded_channel::<u8>();
    let task = tokio::spawn(async move {
        let mut ws = peer.await.unwrap();

        // Subscription A: snapshot, updates seq 1..=2, then terminal state/end.
        let a = recv_frame(&mut ws).await;
        assert_eq!(a["method"], "state/subscribe");
        let a_id = a["params"]["input"]["subscriptionId"]
            .as_str()
            .unwrap()
            .to_string();
        let a_ctx = a["params"]["context"].clone();
        send_text(&mut ws, snapshot_result(&a["id"], &a_id)).await;
        for expected in 1..=4u8 {
            assert_eq!(cmd.recv().await, Some(expected));
            match expected {
                1 => {
                    send_text(
                        &mut ws,
                        update_notice(&a_ctx, &a_id, 1, 9_007_199_254_740_994, 1),
                    )
                    .await
                }
                2 => {
                    send_text(
                        &mut ws,
                        update_notice(&a_ctx, &a_id, 2, 9_007_199_254_740_995, 2),
                    )
                    .await
                }
                3 => send_text(&mut ws, end_notice(&a_ctx, &a_id)).await,
                4 => {
                    // Repeated end and a late update must both be fenced off.
                    send_text(&mut ws, end_notice(&a_ctx, &a_id)).await;
                    send_text(
                        &mut ws,
                        update_notice(&a_ctx, &a_id, 3, 9_007_199_254_740_996, 3),
                    )
                    .await;
                }
                _ => unreachable!(),
            }
            sent_tx.send(expected).unwrap();
        }

        // Subscription B: a repeated seq terminates with SessionLost.
        let b = recv_frame(&mut ws).await;
        assert_eq!(b["method"], "state/subscribe");
        let b_id = b["params"]["input"]["subscriptionId"]
            .as_str()
            .unwrap()
            .to_string();
        let b_ctx = b["params"]["context"].clone();
        send_text(&mut ws, snapshot_result(&b["id"], &b_id)).await;
        for expected in 5..=7u8 {
            assert_eq!(cmd.recv().await, Some(expected));
            match expected {
                5 => {
                    send_text(
                        &mut ws,
                        update_notice(&b_ctx, &b_id, 1, 9_007_199_254_740_994, 1),
                    )
                    .await
                }
                6 => {
                    // Same seq again, higher revision: must terminate, not merge.
                    send_text(
                        &mut ws,
                        update_notice(&b_ctx, &b_id, 1, 9_007_199_254_740_995, 2),
                    )
                    .await;
                    let unsub = recv_frame(&mut ws).await;
                    assert_eq!(unsub["method"], "state/unsubscribe");
                    send_text(
                        &mut ws,
                        json!({"jsonrpc":"2.0","id":unsub["id"],"result":{"removed":true}})
                            .to_string(),
                    )
                    .await;
                }
                7 => {
                    send_text(
                        &mut ws,
                        update_notice(&b_ctx, &b_id, 2, 9_007_199_254_740_996, 3),
                    )
                    .await
                }
                _ => unreachable!(),
            }
            sent_tx.send(expected).unwrap();
        }

        while let Some(Ok(msg)) = ws.next().await {
            if msg.is_close() {
                break;
            }
        }
    });
    (url, task, step, sent)
}
fn delta(sample: &conex_proto::StateSample) -> f64 {
    let value = serde_json::to_value(sample.value.as_ref().expect("sample value")).unwrap();
    value
        .get("delta")
        .and_then(Value::as_f64)
        .expect("sample value delta")
}
#[tokio::test]
async fn accepted_cancel_does_not_override_confirmed_mutation() {
    let count = Arc::new(AtomicUsize::new(0));
    let (url, task, commit) = server(false, count.clone()).await;
    let client = connect(url).await;
    let call = client
        .invoke(target(), contract(), "counter/add", json!({"delta":3}))
        .unwrap();
    commit.await.unwrap();
    assert!(call.control.cancel().await.unwrap().requested);
    assert_eq!(
        call.result::<Value>().await.unwrap(),
        json!({"count":3,"revision":"1"})
    );
    assert_eq!(count.load(Ordering::SeqCst), 3);
    client.close().await;
    task.await.unwrap();
}
#[tokio::test]
async fn disconnect_after_commit_is_unknown_and_not_replayed() {
    let count = Arc::new(AtomicUsize::new(0));
    let (url, task, _) = server(true, count.clone()).await;
    let client = connect(url).await;
    let call = client
        .invoke(target(), contract(), "counter/add", json!({"delta":3}))
        .unwrap();
    let error = call.result::<Value>().await.unwrap_err();
    assert_eq!(error.code(), -32012);
    assert_eq!(error.execution(), "unknown");
    assert_eq!(error.retry(), "never");
    assert_eq!(count.load(Ordering::SeqCst), 3);
    client.close().await;
    task.await.unwrap();
}
#[test]
fn sequence_is_exact_uint64_and_registration_scoped() {
    use conex_sdk::state::Sequence;
    let mut sequence = Sequence::new(u64::MAX - 2);
    sequence.accept(1, u64::MAX - 1).unwrap();
    assert_eq!(sequence.accept(3, u64::MAX).unwrap_err().code(), -32017);
    assert!(conex_proto::capability::state_sample_from_json(&json!({"revision":"18446744073709551616","value":{},"sampledAtMs":"0","quality":"current"})).is_err());
    assert!(
        conex_proto::capability::state_sample_from_json(
            &json!({"revision":"01","value":{},"sampledAtMs":"0","quality":"current"})
        )
        .is_err()
    );
}
#[tokio::test]
async fn subscription_delivers_snapshot_then_updates_and_terminates_once() {
    let (url, task, step, mut sent) = subscribe_peer().await;
    let client = connect(url).await;
    let target = target();
    let contract = state_contract();

    // Snapshot first, then seq-ordered updates carrying exact >2^53 revisions.
    let mut a = client
        .subscribe(
            target.clone(),
            contract.clone(),
            "value",
            json!({"delta": 0}),
        )
        .unwrap();
    match a.next().await.unwrap() {
        StateEvent::Snapshot(sample) => assert_eq!(sample.revision, 9_007_199_254_740_993),
        other => panic!("expected Snapshot first, got {other:?}"),
    }
    step.send(1).unwrap();
    assert_eq!(sent.recv().await, Some(1));
    match a.next().await.unwrap() {
        StateEvent::Update { seq, sample } => assert_eq!(
            (seq, sample.revision, delta(&sample)),
            (1, 9_007_199_254_740_994, 1.0)
        ),
        other => panic!("expected Update 1, got {other:?}"),
    }
    step.send(2).unwrap();
    assert_eq!(sent.recv().await, Some(2));
    match a.next().await.unwrap() {
        StateEvent::Update { seq, sample } => assert_eq!(
            (seq, sample.revision, delta(&sample)),
            (2, 9_007_199_254_740_995, 2.0)
        ),
        other => panic!("expected Update 2, got {other:?}"),
    }

    // Exactly one terminal End carrying the peer's Unavailable error.
    step.send(3).unwrap();
    assert_eq!(sent.recv().await, Some(3));
    match a.next().await.unwrap() {
        StateEvent::End(error) => {
            assert_eq!(error.code(), conex_proto::ErrorCode::Unavailable as i32)
        }
        other => panic!("expected End, got {other:?}"),
    }

    // Repeated/late end and late update were already processed: nothing more.
    step.send(4).unwrap();
    assert_eq!(sent.recv().await, Some(4));
    assert!(a.next().await.is_none());
    assert!(!a.unsubscribe().await.unwrap().removed);
    drop(a);

    // B: a repeated seq terminates the subscription with SessionLost (-32017).
    let mut b = client
        .subscribe(target, contract, "value", json!({"delta": 0}))
        .unwrap();
    match b.next().await.unwrap() {
        StateEvent::Snapshot(sample) => assert_eq!(sample.revision, 9_007_199_254_740_993),
        other => panic!("expected Snapshot first, got {other:?}"),
    }
    step.send(5).unwrap();
    assert_eq!(sent.recv().await, Some(5));
    match b.next().await.unwrap() {
        StateEvent::Update { seq, sample } => assert_eq!(
            (seq, sample.revision, delta(&sample)),
            (1, 9_007_199_254_740_994, 1.0)
        ),
        other => panic!("expected Update 1, got {other:?}"),
    }
    step.send(6).unwrap();
    assert_eq!(sent.recv().await, Some(6));
    match b.next().await.unwrap() {
        StateEvent::End(error) => {
            assert_eq!(error.code(), conex_proto::ErrorCode::SessionLost as i32)
        }
        other => panic!("expected End(-32017), got {other:?}"),
    }

    // A late update after the SessionLost end must not deliver either.
    step.send(7).unwrap();
    assert_eq!(sent.recv().await, Some(7));
    assert!(b.next().await.is_none());
    drop(b);

    client.close().await;
    task.await.unwrap();
}
