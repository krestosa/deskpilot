// File purpose: Compiles and embeds the native Windows resource manifest during the Rust build.
// Function purpose: Starts DeskPilot, parses the invocation, dispatches the selected command, and exits with its result code.
fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_resource::compile("resources/deskpilot.rc", embed_resource::NONE)
            .manifest_required()
            .expect("compile Windows resources");
    }
}
