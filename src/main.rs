fn get_greeting() -> &'static str {
    "Hello, world!"
}

fn main() {
    println!("{}", get_greeting());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greeting_is_hello_world() {
        assert_eq!(get_greeting(), "Hello, world!");
    }
}
