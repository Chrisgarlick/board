// Recompile when the built frontend changes. Without this, a binary built
// before `npm run build` (or before a frontend change) keeps serving the old
// or missing files, because rust-embed only scans web/dist at compile time.
fn main() {
    println!("cargo:rerun-if-changed=../../web/dist");
}
