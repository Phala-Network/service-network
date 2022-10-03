use crate::runtime::AsyncRuntimeContext;
use log::{debug, info};
use phactory_api::prpc::client::Error;
use phactory_api::prpc::PhactoryInfo;
use phactory_api::pruntime_client::PRuntimeClient;
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tokio::sync::RwLock;

#[derive(Debug, Clone, PartialEq)]
pub enum WorkerRuntimeChannelMessage {
    ShouldUpdateInfo(PhactoryInfo),
    ShouldUpdateStatus(WorkerRuntimeStatus),
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkerRuntimeStatus {
    Starting,
    WaitingForBroker,
    PendingProvision,
    Started,
    BrokerFailed,
}

pub type WrappedWorkerRuntime = Arc<RwLock<WorkerRuntime>>;

pub struct WorkerRuntime {
    pub rt_ctx: &'static AsyncRuntimeContext,
    pub prc: &'static PRuntimeClient,
    pub status: WorkerRuntimeStatus,
    pub initial_info: Option<PhactoryInfo>,
}

impl WorkerRuntime {
    pub fn new(rt_ctx: &'static AsyncRuntimeContext, prc: &'static PRuntimeClient) -> Self {
        Self {
            rt_ctx,
            prc,
            status: WorkerRuntimeStatus::Starting,
            initial_info: None,
        }
    }

    pub fn new_wrapped(
        rt_ctx: &'static AsyncRuntimeContext,
        prc: &'static PRuntimeClient,
    ) -> WrappedWorkerRuntime {
        Arc::new(RwLock::new(Self::new(rt_ctx, prc)))
    }

    pub fn handle_update_info(&mut self, info: PhactoryInfo) {
        info!("Updating info from pRuntime: {:?}", &info);
        self.initial_info = Some(info);
    }

    pub fn handle_update_status(&mut self, s: WorkerRuntimeStatus) {
        info!("Worker status from {:?} changed to {:?}", &self.status, &s);
        self.status = s;
    }
}
