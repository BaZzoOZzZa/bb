use std::any::Any;
use std::fmt;
use std::future::Future;

use bb_client::{BbClient, ClientError, RealtimeClient};
use bb_wire::environment::Host;
use bb_wire::interaction::PendingInteraction;
use bb_wire::project::SidebarBootstrapResponse;
use bb_wire::queue::ThreadQueuedMessage;
use bb_wire::realtime::SubscriptionTarget;
use bb_wire::system::SystemAttentionResponse;
use bb_wire::thread::{ThreadGetQuery, ThreadResponse};
use bb_wire::timeline::{
    ThreadTimelineQuery, ThreadTimelineResponse, TurnSummaryDetailsQuery,
    TurnSummaryDetailsResponse,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchError {
    pub message: String,
    pub status: Option<u16>,
    pub code: Option<String>,
    pub retryable: bool,
}

impl FetchError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status: None,
            code: None,
            retryable: false,
        }
    }

    pub fn http(status: u16, code: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            status: Some(status),
            code: code.map(str::to_owned),
            retryable: false,
        }
    }

    pub fn is_not_found(&self) -> bool {
        self.status == Some(404)
    }

    pub fn is_stale_cursor(&self) -> bool {
        self.status == Some(400) && self.code.as_deref() == Some("invalid_request")
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FetchError {}

impl From<ClientError> for FetchError {
    fn from(error: ClientError) -> Self {
        Self {
            message: error.to_string(),
            status: error.status(),
            code: error.code().map(str::to_owned),
            retryable: error.is_retryable(),
        }
    }
}

pub type FetchResult<T> = Result<T, FetchError>;

pub trait Backend: Send + Sync + 'static {
    fn sidebar_bootstrap(
        &self,
    ) -> impl Future<Output = FetchResult<SidebarBootstrapResponse>> + Send;

    fn hosts(&self) -> impl Future<Output = FetchResult<Vec<Host>>> + Send;

    fn attention(&self) -> impl Future<Output = FetchResult<SystemAttentionResponse>> + Send;

    fn thread(&self, thread_id: String)
    -> impl Future<Output = FetchResult<ThreadResponse>> + Send;

    fn timeline(
        &self,
        thread_id: String,
        query: ThreadTimelineQuery,
    ) -> impl Future<Output = FetchResult<ThreadTimelineResponse>> + Send;

    fn turn_summary_details(
        &self,
        thread_id: String,
        query: TurnSummaryDetailsQuery,
    ) -> impl Future<Output = FetchResult<TurnSummaryDetailsResponse>> + Send;

    fn interactions(
        &self,
        thread_id: String,
    ) -> impl Future<Output = FetchResult<Vec<PendingInteraction>>> + Send;

    fn queued_messages(
        &self,
        thread_id: String,
    ) -> impl Future<Output = FetchResult<Vec<ThreadQueuedMessage>>> + Send;
}

impl Backend for BbClient {
    async fn sidebar_bootstrap(&self) -> FetchResult<SidebarBootstrapResponse> {
        Ok(BbClient::sidebar_bootstrap(self).await?)
    }

    async fn hosts(&self) -> FetchResult<Vec<Host>> {
        Ok(self.list_hosts().await?)
    }

    async fn attention(&self) -> FetchResult<SystemAttentionResponse> {
        Ok(self.system_attention().await?)
    }

    async fn thread(&self, thread_id: String) -> FetchResult<ThreadResponse> {
        Ok(self
            .get_thread(&thread_id, &ThreadGetQuery::with_environment_and_host())
            .await?)
    }

    async fn timeline(
        &self,
        thread_id: String,
        query: ThreadTimelineQuery,
    ) -> FetchResult<ThreadTimelineResponse> {
        Ok(self.thread_timeline(&thread_id, &query).await?)
    }

    async fn turn_summary_details(
        &self,
        thread_id: String,
        query: TurnSummaryDetailsQuery,
    ) -> FetchResult<TurnSummaryDetailsResponse> {
        Ok(self.thread_turn_summary_details(&thread_id, &query).await?)
    }

    async fn interactions(&self, thread_id: String) -> FetchResult<Vec<PendingInteraction>> {
        Ok(self.thread_interactions(&thread_id).await?)
    }

    async fn queued_messages(&self, thread_id: String) -> FetchResult<Vec<ThreadQueuedMessage>> {
        Ok(BbClient::queued_messages(self, &thread_id).await?)
    }
}

pub type SubscriptionGuard = Box<dyn Any + Send + Sync>;

pub trait RealtimeSubscriber: Send + Sync + 'static {
    fn subscribe(&self, target: SubscriptionTarget) -> SubscriptionGuard;
}

impl RealtimeSubscriber for RealtimeClient {
    fn subscribe(&self, target: SubscriptionTarget) -> SubscriptionGuard {
        Box::new(RealtimeClient::subscribe(self, target))
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoRealtime;

impl RealtimeSubscriber for NoRealtime {
    fn subscribe(&self, _target: SubscriptionTarget) -> SubscriptionGuard {
        Box::new(())
    }
}
