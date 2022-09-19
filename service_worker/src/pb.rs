pub mod prpc {
    use serde::{Deserialize, Serialize};
    include!(concat!(env!("OUT_DIR"), "/prpc.rs"));
}

pub mod pruntime_rpc {
    use serde::{Deserialize, Serialize};
    include!(concat!(env!("OUT_DIR"), "/pruntime_rpc.rs"));
}
