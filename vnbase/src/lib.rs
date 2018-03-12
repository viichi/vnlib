
pub mod run_loop;

#[cfg(test)]
mod tests {

    use super::*;
    use std::time::Duration;

    #[test]
    fn it_works() {
        run_loop::new_timer()
            .with_callback(run_loop::stop)
            .and_start(Duration::default());

        run_loop::run();
        
    }
}
