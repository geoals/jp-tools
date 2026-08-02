use std::path::Path;
use sudachi::analysis::stateless_tokenizer::StatelessTokenizer;
use sudachi::analysis::{Mode, Tokenize};
use sudachi::config::Config;
use sudachi::dic::dictionary::JapaneseDictionary;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cfg = Config::new(None, None, Some(Path::new(&args[1]).to_path_buf())).unwrap();
    let dict = JapaneseDictionary::from_cfg(&cfg).unwrap();
    let tk = StatelessTokenizer::new(&dict);
    for text in &args[2..] {
        for mode in [Mode::C, Mode::B, Mode::A] {
            print!("{:?} {} :", mode, text);
            for m in tk.tokenize(text, mode, false).unwrap().iter() {
                print!(
                    " [{}|dict={}|norm={}|read={}|pos={:?}|oov={}]",
                    m.surface(),
                    m.dictionary_form(),
                    m.normalized_form(),
                    m.reading_form(),
                    m.part_of_speech(),
                    m.is_oov()
                );
            }
            println!();
        }
    }
}
