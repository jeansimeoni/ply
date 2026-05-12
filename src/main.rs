fn main() {
    if let Err(err) = ply::run() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
