use std::io::Result;
fn main() -> Result<()> {
    let mut prost_build = prost_build::Config::new();
    prost_build.btree_map(&["."]);
    prost_build.type_attribute(
        ".",
        "#[derive(Deserialize, Serialize)] #[serde(rename_all = \"snake_case\")]",
    );
    prost_build.compile_protos(
        &["proto/prpc.proto", "proto/pruntime_rpc.proto"],
        &["proto"],
    )?;
    Ok(())
}
