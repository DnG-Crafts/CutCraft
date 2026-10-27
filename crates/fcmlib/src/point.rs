use crate::encode::Encode;
use nom::combinator::map;
use nom::Parser;
use nom::number::complete::le_i32;
use nom::IResult;
use std::io::Write;

#[derive(Default, Debug, Copy, Clone)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

pub(crate) fn read_point(input: &[u8]) -> IResult<&[u8], Point> {
    map((le_i32, le_i32), |(x, y)| Point { x, y }).parse(input)
}

impl Encode for Point {
    fn encode(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        buffer.write_all(&self.x.to_le_bytes())?;
        buffer.write_all(&self.y.to_le_bytes())?;
        Ok(())
    }
}
