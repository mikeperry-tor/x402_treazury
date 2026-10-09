//! Per-RPC progress, below protobuf decoding and above HTTP/2 multiplexing.
use http_body::{Body as HttpBody, Frame, SizeHint};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tonic::{
    body::Body,
    codegen::{Bytes, Service, http},
    transport::Channel,
};

#[derive(Clone, Copy)]
enum ConnectionState {
    Connecting(tokio::time::Instant),
    Established(tokio::time::Instant),
}

/// Only a connector attempt or first stream dispatch changes this state. Other
/// responses and HTTP/2 keepalives cannot renew a queued RPC's readiness wait.
#[derive(Clone)]
pub(super) struct Readiness {
    state: tokio::sync::watch::Sender<ConnectionState>,
    connect_timeout: Duration,
}
impl Readiness {
    pub(super) fn new(connect_timeout: Duration) -> Self {
        let (state, _) =
            tokio::sync::watch::channel(ConnectionState::Established(tokio::time::Instant::now()));
        Self {
            state,
            connect_timeout,
        }
    }
    pub(super) fn established(&self) {
        self.state.send_if_modified(|state| {
            if matches!(state, ConnectionState::Connecting(_)) {
                *state = ConnectionState::Established(tokio::time::Instant::now());
                true
            } else {
                false
            }
        });
    }
    pub(super) fn connector<C>(&self, inner: C) -> Connector<C> {
        Connector {
            inner,
            readiness: self.clone(),
        }
    }
    async fn stalled(&self, inactivity: Duration) {
        let started = tokio::time::Instant::now();
        let mut state = self.state.subscribe();
        loop {
            let deadline = match *state.borrow_and_update() {
                // Tonic owns the connection timeout (including TLS). Until a
                // stream dispatch proves connection readiness, reserve its full
                // allowance, followed by the stream-capacity inactivity window.
                ConnectionState::Connecting(at) => {
                    started.max(at + self.connect_timeout) + inactivity
                }
                ConnectionState::Established(at) => started.max(at) + inactivity,
            };
            tokio::select! {
                biased;
                _ = state.changed() => {},
                _ = tokio::time::sleep_until(deadline) => return,
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct Connector<C> {
    inner: C,
    readiness: Readiness,
}
impl<C: Service<http::Uri>> Service<http::Uri> for Connector<C> {
    type Response = C::Response;
    type Error = C::Error;
    type Future = C::Future;
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }
    fn call(&mut self, uri: http::Uri) -> Self::Future {
        self.readiness
            .state
            .send_replace(ConnectionState::Connecting(tokio::time::Instant::now()));
        self.inner.call(uri)
    }
}

#[derive(Clone)]
pub(super) struct ProgressChannel {
    inner: Channel,
    inactivity: Duration,
    readiness: Readiness,
}

impl ProgressChannel {
    pub(super) fn new(inner: Channel, inactivity: Duration, readiness: Readiness) -> Self {
        Self {
            inner,
            inactivity,
            readiness,
        }
    }
}

impl Service<http::Request<Body>> for ProgressChannel {
    type Response = http::Response<Body>;
    type Error = tonic::Status;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Channel buffer readiness is awaited inside the owned, bounded dispatch
        // stage too; otherwise a full buffer could bypass the readiness guard.
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: http::Request<Body>) -> Self::Future {
        // Library callers still supply legacy durations. Do not send a total
        // deadline to the server or tonic's channel timeout layer.
        request.headers_mut().remove("grpc-timeout");
        let (sent, dispatched) = tokio::sync::oneshot::channel();
        let cancelled = tokio_util::sync::CancellationToken::new();
        let request = request.map(|inner| {
            Body::new(DispatchBody {
                inner,
                sent: Some(sent),
                readiness: self.readiness.clone(),
                cancelled: cancelled.clone(),
            })
        });
        let mut inner = self.inner.clone();
        let response = async move {
            std::future::poll_fn(|cx| inner.poll_ready(cx)).await?;
            inner.call(request).await
        };
        let inactivity = self.inactivity;
        let readiness = self.readiness.clone();
        Box::pin(async move {
            let cancel_on_drop = cancelled.drop_guard();
            tokio::pin!(response);
            // Channel::call includes its buffered reconnect/readiness wait. Hyper
            // polls the request body only after dispatch on an established stream.
            // Accepted limitation: header waiting includes a progressing upload.
            // The channel exposes no per-stream upload-consumption signal; do
            // not renew this timer using unrelated connection traffic.
            // Connection establishment retains the connector's own policy.
            let response = tokio::select! {
                biased;
                result = &mut response => result,
                result = dispatched => {
                    if result.is_ok() {
                        tokio::time::timeout(inactivity, &mut response)
                            .await
                            .map_err(|_| tonic::Status::deadline_exceeded("grpc_headers_inactivity"))?
                    } else {
                        response.await
                    }
                },
                _ = readiness.stalled(inactivity) => return Err(tonic::Status::deadline_exceeded("grpc_dispatch_inactivity")),
            }.map_err(|e| tonic::Status::from_error(Box::new(e)))?;
            cancel_on_drop.disarm();
            Ok(response.map(|inner| {
                Body::new(ProgressBody {
                    inner,
                    inactivity,
                    timer: Box::pin(tokio::time::sleep(inactivity)),
                    finished: false,
                })
            }))
        })
    }
}

struct DispatchBody {
    inner: Body,
    sent: Option<tokio::sync::oneshot::Sender<()>>,
    readiness: Readiness,
    cancelled: tokio_util::sync::CancellationToken,
}

impl HttpBody for DispatchBody {
    type Data = Bytes;
    type Error = tonic::Status;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        // Hyper can retain a pending-open stream after the response waiter is
        // dropped. If peer capacity returns later, never let it upload a stale
        // RPC body (especially a prepared transaction) on that waiter's behalf.
        if self.cancelled.is_cancelled() {
            return Poll::Ready(Some(Err(tonic::Status::cancelled(
                "grpc_dispatch_cancelled",
            ))));
        }
        if let Some(sent) = self.sent.take() {
            self.readiness.established();
            let _ = sent.send(());
        }
        Pin::new(&mut self.inner).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        // Even an empty body must be polled once to observe actual dispatch.
        self.sent.is_none() && self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

struct ProgressBody {
    inner: Body,
    inactivity: Duration,
    timer: Pin<Box<tokio::time::Sleep>>,
    finished: bool,
}

impl HttpBody for ProgressBody {
    type Data = Bytes;
    type Error = tonic::Status;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        if self.finished {
            return Poll::Ready(None);
        }
        // Poll available data first: time spent with a backpressured consumer
        // is not proof of an idle origin. Only this response's nonempty DATA
        // renews its timer, never pings or progress on a sibling stream.
        match Pin::new(&mut self.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                if frame.data_ref().is_some_and(|data| !data.is_empty()) {
                    let deadline = tokio::time::Instant::now() + self.inactivity;
                    self.timer.as_mut().reset(deadline);
                } else if frame.is_data() && self.timer.as_mut().poll(cx).is_ready() {
                    self.finished = true;
                    return Poll::Ready(Some(Err(tonic::Status::deadline_exceeded(
                        "grpc_body_inactivity",
                    ))));
                }
                return Poll::Ready(Some(Ok(frame)));
            }
            Poll::Ready(result) => {
                self.finished = true;
                return Poll::Ready(result);
            }
            Poll::Pending => {}
        }
        if self.timer.as_mut().poll(cx).is_ready() {
            self.finished = true;
            return Poll::Ready(Some(Err(tonic::Status::deadline_exceeded(
                "grpc_body_inactivity",
            ))));
        }
        Poll::Pending
    }

    fn is_end_stream(&self) -> bool {
        self.finished || self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}
