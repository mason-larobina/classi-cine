use log::*;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

#[derive(clap::ValueEnum, Debug, Clone, Copy)]
pub enum Tokenize {
    Words,
    Chars,
}

#[derive(Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Copy, Clone, Default)]
pub struct Token(u32);

#[derive(Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Copy, Clone, Default)]
pub struct Ngram(u32);

#[derive(Debug)]
pub struct Tokenizer {
    pub tokenize: Tokenize,

    // Token state.
    token_count: u32,
    pub token_string: HashMap<Token, String>,
    string_token: HashMap<String, Token>,

    // Ngram state.
    k: usize,
    pub ngram_count: u32,
    pub ngram_tokens: HashMap<Ngram, Vec<Token>>,
    tokens_ngram: HashMap<Vec<Token>, Ngram>,
}

impl Tokenizer {
    pub fn new(k: usize, tokenize: Tokenize, files: &HashMap<PathBuf, u64>) -> Self {
        assert!(k > 0);

        let file_count = files.len();
        assert!(file_count > 0);

        let mut tokenizer = Self {
            tokenize,

            token_count: 0,
            string_token: HashMap::new(),
            token_string: HashMap::new(),

            k,
            ngram_count: 0,
            ngram_tokens: HashMap::new(),
            tokens_ngram: HashMap::new(),
        };

        // Unique token count per file.
        let mut token_counts: HashMap<String, usize> = HashMap::new();
        for path in files.keys() {
            let mut tokens = tokenizer.tokenize_new(path);
            tokens.sort();
            tokens.dedup();
            for token in tokens {
                let e = token_counts.entry(token).or_default();
                *e += 1;
            }
        }

        let mut unique_tokens: BTreeSet<String> = BTreeSet::new();
        let mut common_tokens: BTreeSet<String> = BTreeSet::new();
        for (token, count) in token_counts {
            if count > 1 {
                tokenizer.make_token(&token);
            } else if count == 1 {
                unique_tokens.insert(token);
            } else if count == file_count {
                common_tokens.insert(token);
            }
        }
        //debug!("Drop unique tokens: {:?}", unique_tokens);
        //debug!("Drop common tokens: {:?}", common_tokens);

        let mut ngram_counts: HashMap<Vec<Token>, usize> = HashMap::new();
        for path in files.keys() {
            let ngrams: BTreeSet<Vec<Token>> = tokenizer.ngrams_new(path).into_iter().collect();
            for ngram in ngrams {
                let e = ngram_counts.entry(ngram).or_default();
                *e += 1;
            }
        }

        let mut unique_ngrams: BTreeSet<Vec<Token>> = BTreeSet::new();
        let mut common_ngrams: BTreeSet<Vec<Token>> = BTreeSet::new();
        for (ngram, count) in ngram_counts {
            if count > 1 {
                tokenizer.make_ngram(&ngram);
            } else if count == 1 {
                unique_ngrams.insert(ngram);
            } else if count == file_count {
                common_ngrams.insert(ngram);
            }
        }
        //debug!("Drop unique ngrams: {:?}", unique_ngrams);
        //debug!("Drop common ngrams: {:?}", common_ngrams);

        info!("File count: {}", file_count);
        info!("Token count: {}", tokenizer.token_count);
        info!("Ngram count: {}", tokenizer.ngram_count);

        tokenizer
    }

    fn make_token(&mut self, s: &str) -> Token {
        if let Some(token) = self.string_token.get(s) {
            return *token;
        }

        self.token_count += 1;
        let token = Token(self.token_count);

        self.string_token.insert(s.to_string(), token);
        self.token_string.insert(token, s.to_string());

        token
    }

    fn make_ngram(&mut self, tokens: &[Token]) -> Ngram {
        if let Some(ngram) = self.tokens_ngram.get(tokens) {
            return *ngram;
        }

        self.ngram_count += 1;
        let ngram = Ngram(self.ngram_count);

        self.tokens_ngram.insert(tokens.to_vec(), ngram);
        self.ngram_tokens.insert(ngram, tokens.to_vec());

        ngram
    }

    fn tokenize_new(&self, path: &Path) -> Vec<String> {
        let mut path: String = path.to_string_lossy().to_string();
        path.make_ascii_lowercase();

        let mut ret = Vec::new();
        match self.tokenize {
            Tokenize::Words => {
                for token in path
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|word| !word.is_empty())
                {
                    ret.push(token.to_string());
                }
            }
            Tokenize::Chars => {
                for c in path.chars() {
                    if c.is_alphanumeric() || c == '/' {
                        ret.push(c.into());
                        continue;
                    } else if Some(" ") != ret.last().map(|x| x.as_str()) {
                        ret.push(' '.into());
                    }
                }
            }
        }
        ret
    }

    pub fn tokenize_cached(&self, path: &Path) -> Vec<Token> {
        let mut ret = Vec::new();
        for token in self.tokenize_new(path) {
            ret.push(self.string_token.get(&token).cloned().unwrap_or_default());
        }
        ret
    }

    fn ngrams_new(&self, path: &Path) -> Vec<Vec<Token>> {
        let tokens = self.tokenize_cached(path);
        let mut ret = Vec::new();

        let j = match self.tokenize {
            Tokenize::Words => 0,
            Tokenize::Chars => 1,
        };
        for i in j..self.k {
            for w in tokens.windows(i + 1) {
                let mut w: Vec<Token> = w.to_vec();
                w.shrink_to_fit();
                ret.push(w);
            }
        }
        ret
    }

    pub fn ngrams_cached(&self, path: &Path) -> Vec<Ngram> {
        let mut ret = Vec::new();
        for ngram in self.ngrams_new(path) {
            ret.push(self.tokens_ngram.get(&ngram).cloned().unwrap_or_default());
        }
        ret
    }
}
