// build.rs
fn main() {
    let protoc_path = protoc_bin_vendored::protoc_bin_path().expect("download protoc");
    std::env::set_var("PROTOC", protoc_path);

    let protos: Vec<_> = std::fs::read_dir("proto")
        .expect("proto dir")
        .filter_map(|e| {
            let p = e.ok()?.path();
            (p.extension()?.to_str()? == "proto").then_some(p)
        })
        .collect();

    prost_build::Config::new()
        .include_file("mexc.pb.rs")
        .compile_protos(&protos, &["proto"])
        .expect("compile protos");

    println!("cargo:rerun-if-changed=proto");
}
