use futures::future::try_join_all;
use log::error;
use tokio::task::JoinHandle;

pub static CONTENT_TYPE_JSON: &'static str = "application/json";

pub async fn join_handles(handles: Vec<JoinHandle<()>>) {
    match try_join_all(handles).await {
        Ok(_) => {
            std::process::exit(0);
        }
        Err(err) => {
            error!("Fatal error: {}", err);
            std::process::exit(100);
        }
    }
}
