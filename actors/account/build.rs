fn main() {
    #[cfg(feature = "wasm")]
    fil_builtin_actors_builder::build().expect("failed to build wasm binary");
}
