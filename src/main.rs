fn main() {
    if let Err(err) = ply::run() {
        ply::print_error(&err);
        std::process::exit(1);
    }
}
