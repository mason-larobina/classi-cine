use std::collections::{HashSet, HashMap};
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use crate::bloom::{Bloom, IntoMask};

#[derive(Default, Debug, Eq, PartialEq, Copy, Clone, Ord, PartialOrd, Hash)]
pub struct Token(pub(crate) u32);

#[derive(Debug, PartialEq, Eq, Copy, Clone, Ord, PartialOrd, Hash)]
pub struct Pair(pub Token, pub Token);

impl Pair {
    pub(crate) fn to_string(&self, map: &TokenMap) -> String {
        let mut s = String::new();
        s.push_str(map.get_str(self.0).unwrap());
        s.push_str(map.get_str(self.1).unwrap());
        s
    }
}

impl IntoMask for Pair {
    fn into_mask(&self) -> u128 {
        Bloom::mask(self)
    }
}

#[derive(Debug, Default)]
pub struct Tokens {
    bloom: Bloom,
    tokens: Vec<Token>,
}

impl Tokens {
    fn calc_bloom(&mut self) {
        let mut bloom = Bloom::default();
        for pair in self.pairs() {
            bloom.set(pair);
        }
        self.bloom = bloom;
    }

    pub fn from_str_and_create(s: &str, token_map: &mut TokenMap) -> Tokens {
        let mut tokens = Tokens::default();
        let mut tmp_string = String::new();
        for c in s.chars() {
            tmp_string.clear();
            tmp_string.push(c);
            let token = token_map.get_or_create_token(&tmp_string);
            tokens.tokens.push(token);
        }
        tokens.calc_bloom();
        tokens
    }

    pub fn from_str_or_unknown(s: &str, token_map: &TokenMap) -> Tokens {
        let mut tokens = Tokens::default();
        let mut tmp_string = String::new();
        for c in s.chars() {
            tmp_string.clear();
            tmp_string.push(c);
            let token = token_map.get_token(&tmp_string);
            tokens.tokens.push(token);
        }
        tokens.calc_bloom();
        tokens
    }

    pub fn from_replace(&mut self, from: &Tokens, Pair(a, b): Pair, merged: Token, skip: &[Token]) -> bool {
        self.tokens.clear();
        let mut i = 0;
        let len = from.len();
        while i < len {
            let token_a = from.tokens[i];
            if a == token_a && (i + 1) < len {
                let token_b = from.tokens[i + 1];
                if b == token_b && !skip.contains(&token_a) && !skip.contains(&token_b) {
                    self.tokens.push(merged);
                    i += 2;
                    continue;
                }
            }
            self.tokens.push(token_a);
            i += 1;
        }
        self.calc_bloom();
        self.tokens.len() != from.tokens.len()
    }

    pub fn contains<M: IntoMask>(&self, e: &M) -> bool {
        self.bloom.contains(e)
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn pairs<'a>(&'a self) -> impl Iterator<Item = Pair> + 'a {
        self.tokens.windows(2).map(|w| Pair(w[0], w[1]))
    }

    pub fn as_slice<'a>(&'a self) -> &'a [Token] {
        self.tokens.as_slice()
    }

    pub fn debug_strs<'a>(&'a self, map: &'a TokenMap) -> Vec<&'a str> {
        let mut comps = Vec::new();
        for t in &self.tokens {
            comps.push(map.get_str(*t).unwrap());
        }
        comps
    }	
}

#[derive(Debug, Clone)]
pub struct TokenMap {
    str_token: HashMap<String, Token>,
    token_str: HashMap<Token, String>,
    unknown: Token,
}

impl TokenMap {
    pub fn new() -> Self {
        let mut token_map = Self {
            str_token: Default::default(),
            token_str: Default::default(),
            unknown: Default::default(),
        };
        let unknown = token_map.create_token("<UNK>");
        assert_eq!(token_map.unknown, unknown);
        token_map
    }

    pub fn create_token(&mut self, s: &str) -> Token {
        let token = Token(self.str_token.len().try_into().unwrap());
        assert!(self.str_token.insert(s.to_string(), token.clone()).is_none());
        assert!(self.token_str.insert(token.clone(), s.to_string()).is_none());
        token
    }

    pub fn get_or_create_token(&mut self, s: &str) -> Token {
        if let Some(token) = self.str_token.get(s) {
            return *token;
        } else {
            self.create_token(s)
        }
    }

    pub fn merge(&mut self, Pair(a, b): Pair) -> Token {
        let a = self.get_str(a).unwrap();
        let b = self.get_str(b).unwrap();
        let mut merged = String::with_capacity(a.len() + b.len());
        merged.push_str(a);
        merged.push_str(b);
        self.create_token(&merged)
    }

    pub fn get_token(&self, s: &str) -> Token {
        self.str_token.get(s).copied().unwrap_or_default()
    }

    pub fn get_str<'a>(&'a self, token: Token) -> Option<&'a str> {
        self.token_str.get(&token).map(String::as_str)
    }
}
