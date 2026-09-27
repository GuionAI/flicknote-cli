fn main() {
    let include = std::env::var("DEP_SQLITE3_INCLUDE")
        .expect("libsqlite3-sys must expose the bundled SQLite headers");
    cc::Build::new()
        .file("vendor/better-trigram/better-trigram.c")
        .include("vendor/better-trigram")
        .include(include)
        .opt_level(1)
        .flag_if_supported("-std=c99")
        .compile("better_trigram");
    println!("cargo:rerun-if-changed=vendor/better-trigram");
}
