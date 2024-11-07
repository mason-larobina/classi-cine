use std::collections::{HashSet, HashMap};
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;

#[derive(Debug, Eq, PartialEq, Copy, Clone, Ord, PartialOrd, Hash)]
pub struct Token(pub(crate) u32);

#[derive(Debug, PartialEq, Eq, Copy, Clone, Ord, PartialOrd, Hash)]
pub struct Pair(pub Token, pub Token);

impl Pair {
    fn mask(&self) -> u128 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        let hash: u64 = hasher.finish();
        return 1 << (hash % 128);
    }

    pub(crate) fn to_string(&self, map: &TokenMap) -> String {
        let mut s = String::new();
        s.push_str(map.get_str(self.0));
        s.push_str(map.get_str(self.1));
        s
    }
}

#[derive(Debug, PartialEq, Eq, Default)]
pub struct Tokens {
    pair_mask: u128,
    tokens: Vec<Token>,
}

impl Tokens {
    fn calc_pair_mask(&mut self) {
        let mut pair_mask = 0;
        for pair in self.pairs() {
            pair_mask |= pair.mask();
        }
        self.pair_mask = pair_mask;
    }

    pub fn pair_mask_bits(&self) -> u32 {
        self.pair_mask.count_ones()
    }

    pub fn from_str(s: &str, token_map: &mut TokenMap) -> Tokens {
        let mut tokens = Tokens::default();
        let mut tmp_string = String::new();
        for c in s.chars() {
            tmp_string.clear();
            tmp_string.push(c);
            let token = token_map.get_or_create_token(&tmp_string);
            tokens.tokens.push(token);
        }
        tokens.calc_pair_mask();
        tokens
    }

    pub fn from_replace(&mut self, from: &Tokens, Pair(a, b): Pair, c: Token) -> bool {
        self.tokens.clear();
        let mut i = 0;
        let len = from.len();
        while i < len {
            let t = unsafe { *from.tokens.get_unchecked(i) };
            if a == t && (i + 1) < len && b == unsafe { *from.tokens.get_unchecked(i + 1) } {
                self.tokens.push(c);
                i += 2;
            } else {
                self.tokens.push(t);
                i += 1;
            }
        }
        self.calc_pair_mask();
        self.tokens.len() != from.tokens.len()
    }

    pub fn contains(&self, pair: &Pair) -> bool {
        (self.pair_mask & pair.mask()) != 0
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

    pub fn to_string(&self, map: &TokenMap) -> String {
        let mut s = String::new();
        for t in &self.tokens {
            s.push_str(map.get_str(*t));
        }
        s
    }	
}

#[derive(Default, Debug, Clone)]
pub struct TokenMap {
    str_token: HashMap<String, Token>,
    token_str: HashMap<Token, String>,
}

impl TokenMap {
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

    pub fn merge_create_token(&mut self, Pair(a, b): Pair) -> Token {
        let s = format!("{}{}", self.get_str(a), self.get_str(b));
        self.create_token(&s)
    }

    pub fn get_token(&self, s: &str) -> Token {
        *self.str_token.get(s).unwrap()
    }

    pub fn get_str<'a>(&'a self, token: Token) -> &'a str {
        self.token_str.get(&token).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_creation() {
        let mut map = TokenMap::default();
        let t1 = map.create_token("a");
        let t2 = map.create_token("b");
        
        assert_eq!(map.get_str(t1), "a");
        assert_eq!(map.get_str(t2), "b");
        assert_eq!(map.get_token("a"), t1);
        assert_eq!(map.get_token("b"), t2);
    }

    #[test]
    fn test_get_or_create_token() {
        let mut map = TokenMap::default();
        let t1 = map.get_or_create_token("a");
        let t2 = map.get_or_create_token("a");
        assert_eq!(t1, t2);
        
        let t3 = map.get_or_create_token("b");
        assert_ne!(t1, t3);
    }

    #[test]
    fn test_tokens_from_str() {
        let mut map = TokenMap::default();
        let tokens = Tokens::from_str("hello", &mut map);
        
        assert_eq!(tokens.len(), 5);
        assert_eq!(map.get_str(tokens.as_slice()[0]), "h");
        assert_eq!(map.get_str(tokens.as_slice()[4]), "o");
        assert_eq!(tokens.to_string(&map), String::from("hello"));
    }

    #[test]
    fn test_pair_masking() {
        let mut map = TokenMap::default();
        let tokens = Tokens::from_str("ab", &mut map);
        let pair = Pair(map.get_token("a"), map.get_token("b"));
        
        assert!(tokens.contains(&pair));
        
        let non_existing_pair = Pair(map.get_token("a"), map.get_token("a"));
        assert!(!tokens.contains(&non_existing_pair));
    }

    #[test]
    fn test_from_replace() {
        let mut map = TokenMap::default();
        let original = Tokens::from_str("hello", &mut map);
        
        let h = map.get_token("h");
        let e = map.get_token("e");
        let x = map.create_token("x");
        
        let mut new = Tokens::default();
        new.from_replace(&original, Pair(h, e), x);

        assert_eq!(new.to_string(&map), String::from("xllo"));
    }

    #[test]
    fn test_merge_create_token() {
        let mut map = TokenMap::default();
        let t1 = map.create_token("a");
        let t2 = map.create_token("b");
        let merged = map.merge_create_token(t1, t2);
        
        assert_eq!(map.get_str(merged), "ab");
    }

    #[test]
    fn test_pairs_iterator() {
        let mut map = TokenMap::default();
        let tokens = Tokens::from_str("abc", &mut map);
        let pairs: Vec<String> = tokens.pairs().map(|p| p.to_string(&map)).collect();
        assert_eq!(pairs, vec![String::from("ab"), String::from("bc")]);
    }

    #[test]
    #[should_panic]
    fn test_invalid_token_access() {
        let map = TokenMap::default();
        let invalid_token = Token(999);
        map.get_str(invalid_token);
    }

    #[test]
    fn test_empty_string() {
        let mut map = TokenMap::default();
        let tokens = Tokens::from_str("", &mut map);
        assert_eq!(tokens.len(), 0);
        assert_eq!(tokens.pairs().count(), 0);
    }

    #[test]
    fn test_single_char() {
        let mut map = TokenMap::default();
        let tokens = Tokens::from_str("a", &mut map);
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens.pairs().count(), 0);
    }

    #[test]
    fn test_pair_mask_collision_resistance() {
        let mut map = TokenMap::default();
        let mut seen_masks = HashSet::new();
        
        // Test a variety of pairs to ensure mask collisions are rare
        for c1 in 'a'..='z' {
            for c2 in 'a'..='z' {
                let t1 = map.get_or_create_token(&c1.to_string());
                let t2 = map.get_or_create_token(&c2.to_string());
                let pair = Pair(t1, t2);
                let mask = pair.mask();
                seen_masks.insert(mask);
            }
        }
        
        // We should have a reasonable distribution of masks
        assert!(seen_masks.len() > 20); // At least 20 unique masks out of 676 pairs
    }
}
