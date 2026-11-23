fn main() {
    println!("cargo:rerun-if-changed=fuzzie.rc");
    println!("cargo:rerun-if-changed=assets/fuzzie.ico");
    embed_resource::compile("fuzzie.rc", embed_resource::NONE).manifest_optional().unwrap();
}
