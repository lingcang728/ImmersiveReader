use std::error::Error;
use std::io::Write;
use std::time::Duration;

fn main() -> Result<(), Box<dyn Error>> {
    let book_id = std::env::args()
        .nth(1)
        .ok_or("usage: cargo run --example reader_qa -- <book-id>")?;
    let reader = immersive_reader_lib::start_standalone_reader(&book_id)?;
    println!("{}", reader.url());
    std::io::stdout().flush()?;
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}
