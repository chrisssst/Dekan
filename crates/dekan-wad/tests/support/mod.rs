use dekan_wad::prop::tree::{Field, Value};

pub const STRING: u8 = 16;
pub const FIXED: [(u8, usize); 20] = [
    (0, 0),
    (1, 1),
    (2, 1),
    (3, 1),
    (4, 2),
    (5, 2),
    (6, 4),
    (7, 4),
    (8, 8),
    (9, 8),
    (10, 4),
    (11, 8),
    (12, 12),
    (13, 16),
    (14, 64),
    (15, 4),
    (17, 4),
    (18, 8),
    (0x84, 4),
    (0x87, 1),
];

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }

    pub fn text(&mut self, max: u64) -> String {
        const ALPHABET: &[u8] = b"abcdefXYZ_/.0123456789";
        (0..self.below(max) + 1)
            .map(|_| ALPHABET[self.below(ALPHABET.len() as u64) as usize] as char)
            .collect()
    }
}

pub fn encoded_string(text: &str) -> Vec<u8> {
    let mut bytes = u16::try_from(text.len())
        .expect("short")
        .to_le_bytes()
        .to_vec();
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

pub fn primitive(rng: &mut Rng, kind: u8) -> Value {
    let bytes = if kind == STRING {
        encoded_string(&rng.text(40))
    } else {
        let size = FIXED
            .iter()
            .find(|(k, _)| *k == kind)
            .expect("fixed kind")
            .1;
        rng.bytes(size)
    };
    Value::Raw { kind, bytes }
}

pub fn primitive_kind(rng: &mut Rng) -> u8 {
    if rng.chance(25) {
        STRING
    } else {
        FIXED[rng.below(FIXED.len() as u64) as usize].0
    }
}

pub fn fields(rng: &mut Rng, depth: u32) -> Vec<Field> {
    (0..rng.below(6))
        .map(|_| Field {
            name: rng.next() as u32,
            value: value(rng, depth),
        })
        .collect()
}

pub fn structure(rng: &mut Rng, depth: u32) -> Value {
    let kind = if rng.chance(50) { 0x82 } else { 0x83 };
    if rng.chance(10) {
        return Value::Struct {
            kind,
            class: 0,
            fields: Vec::new(),
        };
    }
    Value::Struct {
        kind,
        class: (rng.next() as u32).max(1),
        fields: fields(rng, depth + 1),
    }
}

pub fn value(rng: &mut Rng, depth: u32) -> Value {
    if depth >= 4 || rng.chance(55) {
        let kind = primitive_kind(rng);
        return primitive(rng, kind);
    }
    match rng.below(4) {
        0 => {
            let element = if rng.chance(30) {
                0x83
            } else {
                primitive_kind(rng)
            };
            let items = (0..rng.below(5))
                .map(|_| {
                    if element == 0x83 {
                        Value::Struct {
                            kind: 0x83,
                            class: (rng.next() as u32).max(1),
                            fields: fields(rng, depth + 1),
                        }
                    } else {
                        primitive(rng, element)
                    }
                })
                .collect();
            Value::List {
                kind: if rng.chance(50) { 0x80 } else { 0x81 },
                element,
                items,
            }
        }
        1 => structure(rng, depth),
        2 => {
            let inner = primitive_kind(rng);
            Value::Optional {
                inner,
                value: rng.chance(60).then(|| Box::new(primitive(rng, inner))),
            }
        }
        _ => {
            let key = [7u8, 17, STRING][rng.below(3) as usize];
            let value_kind = primitive_kind(rng);
            let entries = (0..rng.below(5))
                .map(|_| (primitive(rng, key), primitive(rng, value_kind)))
                .collect();
            Value::Map {
                key,
                value: value_kind,
                entries,
            }
        }
    }
}
