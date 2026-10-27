//! 문법 강조 예시 (Rust)
use std::collections::HashMap;

/// 단어 수를 센다
fn count<'a>(text: &'a str) -> HashMap<&'a str, usize> {
    let mut map = HashMap::new();
    for word in text.split_whitespace() {
        *map.entry(word).or_insert(0) += 1;
    }
    map
}

fn main() {
    let text = "가 나 가 \"따옴표\"";
    let sep = '\n';
    /* 블록 주석 */
    let total: u32 = 0x1F + 1_000;
    println!("{:?}{sep}{total}", count(text));
}
