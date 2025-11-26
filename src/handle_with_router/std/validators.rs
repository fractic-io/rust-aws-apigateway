pub trait Verifier: Send + Sync {
    fn name(&self) -> &'static str;
}
