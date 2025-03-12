use crate::bloom::{Bloom, IntoMask};
use std::collections::HashMap;
use std::hash::Hash;

/// A token representing a unique string in the vocabulary
#[derive(Default, Debug, Eq, PartialEq, Copy, Clone, Ord, PartialOrd, Hash)]
pub struct Token(pub(crate) u32);

#[derive(Debug, PartialEq, Eq, Copy, Clone, Ord, PartialOrd, Hash)]
pub struct Pair(pub Token, pub Token);

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
    fn calc_bloom(&mut self, token_map: &TokenMap) {
        let mut bloom = Bloom::default();
        for pair in self.pairs(token_map) {
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
        tokens.calc_bloom(&token_map);
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
        tokens.calc_bloom(token_map);
        tokens
    }

    pub fn from_replace(
        &mut self,
        token_map: &TokenMap,
        from: &Tokens,
        Pair(a, b): Pair,
        merged: Token,
    ) -> bool {
        self.tokens.clear();
        let mut i = 0;
        let len = from.len();
        while i < len {
            let token_a = from.tokens[i];
            if a == token_a && (i + 1) < len {
                let token_b = from.tokens[i + 1];
                if b == token_b {
                    self.tokens.push(merged);
                    i += 2;
                    continue;
                }
            }
            self.tokens.push(token_a);
            i += 1;
        }
        self.calc_bloom(token_map);
        self.tokens.len() != from.tokens.len()
    }

    pub fn contains<M: IntoMask>(&self, e: &M) -> bool {
        self.bloom.contains(e)
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn as_slice<'a>(&'a self) -> &'a [Token] {
        self.tokens.as_slice()
    }

    pub fn pairs<'a>(&'a self, map: &'a TokenMap) -> impl Iterator<Item = Pair> + 'a {
        let last = map.last_special();
        self.tokens.windows(2).filter_map(move |w| {
            let (a, b) = (w[0], w[1]);
            if a > last && b > last {
                Some(Pair(a, b))
            } else {
                None
            }
        })
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
    special: Vec<Token>,
}

impl TokenMap {
    pub fn new(special_chars: &str) -> Self {
        let mut token_map = Self {
            str_token: Default::default(),
            token_str: Default::default(),
            special: Vec::new(),
        };

        // Create the unknown token which is used for any unseen character in the training data.
        let unknown = token_map.create_token("<UNK>");
        assert_eq!(unknown, Token::default());
        token_map.special.push(unknown);

        // Create the special tokens which are not merged in the PairTokenizer.
        for c in special_chars.chars() {
            let t = token_map.get_or_create_token(&c.to_string());
            token_map.special.push(t);
        }

        token_map
    }

    pub fn last_special(&self) -> Token {
        *self.special.last().unwrap()
    }

    pub fn create_token(&mut self, s: &str) -> Token {
        let token = Token(self.str_token.len().try_into().unwrap());
        assert!(
            self.str_token
                .insert(s.to_string(), token.clone())
                .is_none()
        );
        assert!(
            self.token_str
                .insert(token.clone(), s.to_string())
                .is_none()
        );
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

    pub fn count(&self) -> usize {
        self.token_str.len()
    }
}
