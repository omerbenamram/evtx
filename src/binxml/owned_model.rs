use num_traits::Num;

#[derive(Debug)]
pub enum Value<N: Num> {
    String(String),
    Number(N)
}

#[derive(Debug)]
pub struct Attribute<N: Num> {
    key: String,
    value: Value<N>
}

#[derive(Debug)]
pub struct Element<N: Num> {
    attributes: Vec<Attribute<N>>,
    data: Option<Value<N>>
}