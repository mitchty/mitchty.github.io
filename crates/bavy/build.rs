fn main() {
    println!("cargo::rustc-check-cfg=cfg(dev_build)");

    let profile = std::env::var("CARGO_PROFILE").unwrap_or_else(|_| String::from("dev"));
    if !profile.starts_with("release") {
        println!("cargo:rustc-cfg=dev_build");
    }
}
