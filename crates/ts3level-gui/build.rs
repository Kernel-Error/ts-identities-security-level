fn main() {
    glib_build_tools::compile_resources(
        &["data/resources"],
        "data/resources/resources.gresource.xml",
        "ts3level.gresource",
    );
}
