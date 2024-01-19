use std::collections::HashMap;

#[derive(Hash, Eq, PartialEq, Debug, Copy, Clone, Ord, PartialOrd)]
pub struct Token {
    id: u32,
    len: u32,
}

#[test]
fn token_sizeof_test() {
    assert_eq!(std::mem::size_of::<Token>(), 8);
}

impl Token {
    fn new(id: u32, len: u32) -> Self {
        Self { id, len }
    }

    fn new_special(id: u32) -> Self {
        Self::new(id, 0)
    }

    pub fn is_special(&self) -> bool {
        self.len == 0
    }

    pub fn len(&self) -> u32 {
        self.len
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Tokens(Vec<Token>);

impl Tokens {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn push(&mut self, token: Token) {
        self.0.push(token);
    }

    fn clear(&mut self) {
        self.0.clear();
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn pairs<'a>(&'a self) -> impl Iterator<Item = (Token, Token)> + 'a {
        self.0.windows(2).map(|pair| (pair[0], pair[1]))
    }

    pub fn as_slice<'a>(&'a self) -> &'a [Token] {
        self.0.as_slice()
    }

    pub fn replace_new(&self, a: Token, b: Token, c: Token, new: &mut Tokens) {
        new.clear();
        let mut i = 0;
        let len = self.0.len();
        while i < len {
            let t = unsafe { *self.0.get_unchecked(i) };
            if a == t && (i + 1) < len && b == unsafe { *self.0.get_unchecked(i + 1) } {
                new.0.push(c);
                i += 2;
            } else {
                new.0.push(t);
                i += 1;
            }
        }
    }

    pub fn swap(&mut self, other: &mut Tokens) {
        std::mem::swap(&mut self.0, &mut other.0);
    }
}

#[test]
fn tokens_test() {
    let mut vocab = Vocab::new();
    vocab.insert_special(" ");

    let mut tokens = Tokens::new();
    let mut new_tokens = Tokens::new();

    {
        let mut s = String::new();
        for c in "hello world".chars() {
            s.clear();
            s.push(c);
            let token = vocab.insert(&s);
            tokens.push(token);
        }
        assert_eq!(
            tokens,
            Tokens(vec![
                Token { id: 1, len: 1 },
                Token { id: 2, len: 1 },
                Token { id: 3, len: 1 },
                Token { id: 3, len: 1 },
                Token { id: 4, len: 1 },
                Token { id: 0, len: 0 },
                Token { id: 5, len: 1 },
                Token { id: 4, len: 1 },
                Token { id: 6, len: 1 },
                Token { id: 3, len: 1 },
                Token { id: 7, len: 1 },
            ])
        );
    }

    {
        let a = vocab.get_token("h");
        let b = vocab.get_token("e");
        let c = vocab.insert("he");
        tokens.replace_new(a, b, c, &mut new_tokens);
        assert_eq!(
            new_tokens,
            Tokens(vec![
                Token { id: 8, len: 2 },
                Token { id: 3, len: 1 },
                Token { id: 3, len: 1 },
                Token { id: 4, len: 1 },
                Token { id: 0, len: 0 },
                Token { id: 5, len: 1 },
                Token { id: 4, len: 1 },
                Token { id: 6, len: 1 },
                Token { id: 3, len: 1 },
                Token { id: 7, len: 1 },
            ])
        );
        tokens.swap(&mut new_tokens);
    }

    {
        let a = vocab.get_token("l");
        let b = vocab.get_token("d");
        let c = vocab.insert("ld");
        tokens.replace_new(a, b, c, &mut new_tokens);
        assert_eq!(
            new_tokens,
            Tokens(vec![
                Token { id: 8, len: 2 },
                Token { id: 3, len: 1 },
                Token { id: 3, len: 1 },
                Token { id: 4, len: 1 },
                Token { id: 0, len: 0 },
                Token { id: 5, len: 1 },
                Token { id: 4, len: 1 },
                Token { id: 6, len: 1 },
                Token { id: 9, len: 2 },
            ])
        );
        tokens.swap(&mut new_tokens);
    }

    {
        let a = vocab.get_token("he");
        let b = vocab.get_token("l");
        let c = vocab.insert("hel");
        tokens.replace_new(a, b, c, &mut new_tokens);
        assert_eq!(
            new_tokens,
            Tokens(vec![
                Token { id: 10, len: 3 },
                Token { id: 3, len: 1 },
                Token { id: 4, len: 1 },
                Token { id: 0, len: 0 },
                Token { id: 5, len: 1 },
                Token { id: 4, len: 1 },
                Token { id: 6, len: 1 },
                Token { id: 9, len: 2 },
            ])
        );
        std::mem::swap(&mut tokens, &mut new_tokens);
    }
}

#[derive(Default)]
pub struct Vocab {
    str_token: HashMap<String, Token>,
    token_str: HashMap<Token, String>,
}

impl Vocab {
    pub fn new() -> Vocab {
        Vocab {
            str_token: HashMap::new(),
            token_str: HashMap::new(),
        }
    }

    fn insert_return(&mut self, s: &str, token: Token) -> Token {
        self.str_token.insert(s.to_string(), token);
        self.token_str.insert(token, s.to_string());
        assert_eq!(self.str_token.len(), self.token_str.len());
        token
    }

    pub fn insert_special(&mut self, s: &str) -> Token {
        if let Some(token) = self.str_token.get(s) {
            return *token;
        }
        let token = Token::new_special(self.str_token.len().try_into().unwrap());
        self.insert_return(s, token)
    }

    pub fn insert(&mut self, s: &str) -> Token {
        if let Some(token) = self.str_token.get(s) {
            return *token;
        }
        let token = Token::new(
            self.str_token.len().try_into().unwrap(),
            s.chars().count().try_into().unwrap(),
        );
        self.insert_return(s, token)
    }

    pub fn get_token(&self, s: &str) -> Token {
        *self.str_token.get(s).unwrap()
    }

    pub fn get_str<'a>(&'a self, token: Token) -> &'a str {
        self.token_str.get(&token).unwrap()
    }
}
