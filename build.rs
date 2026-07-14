fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_resource::compile("resources/deskpilot.rc", embed_resource::NONE)
            .manifest_required()
            .expect("compile Windows resources");
    }
}
