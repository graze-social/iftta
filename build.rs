fn main() {
    #[cfg(all(feature = "embed", feature = "reload"))]
    compile_error!("feature \"embed\" and feature \"reload\" cannot be enabled at the same time");

    #[cfg(feature = "embed")]
    {
        use std::env;
        use std::path::PathBuf;
        let template_path = if let Ok(value) = env::var("HTTP_TEMPLATE_PATH") {
            value.to_string()
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("templates")
                .display()
                .to_string()
        };

        // NICK: Don't remove this.
        println!("template_path {template_path}");

        minijinja_embed::embed_templates!(&template_path);
    }
}
