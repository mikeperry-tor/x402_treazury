fn main() {
    println!(
        "{}",
        protoc_bin_vendored::protoc_bin_path()
            .expect("supported platform")
            .display()
    );
}
