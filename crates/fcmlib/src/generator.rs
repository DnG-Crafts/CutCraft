use std::io::Write;

use nom::branch::alt;
use nom::Parser;
use nom::bytes::complete::tag;
use nom::combinator::map;
use nom::number::complete::le_u32;
use nom::IResult;

use crate::encode::Encode;

#[derive(Debug)]
pub enum Generator {
    App(u32),
    Web(u32),
    Device(u32, u32),
}

pub(crate) fn read_generator(input: &[u8]) -> IResult<&[u8], Generator> {
    alt((
        read_generator_app,
        read_generator_web,
        read_generator_device,
    )).parse(input)
}

fn read_generator_app(input: &[u8]) -> IResult<&[u8], Generator> {
    map((tag("1APP"), le_u32), |(_, version)| {
        Generator::App(version)
    }).parse(input)
}

fn read_generator_web(input: &[u8]) -> IResult<&[u8], Generator> {
    map((tag("1WEB"), le_u32), |(_, version)| {
        Generator::Web(version)
    }).parse(input)
}

fn read_generator_device(input: &[u8]) -> IResult<&[u8], Generator> {
    map((le_u32, le_u32), |(device, version)| {
        Generator::Device(device, version)
    }).parse(input)
}

impl Encode for Generator {
    fn encode(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        match self {
            Generator::App(version) => {
                buffer.write_all("1APP".as_bytes())?;
                version.encode(buffer)?;
            }
            Generator::Web(version) => {
                buffer.write_all("1WEB".as_bytes())?;
                version.encode(buffer)?;
            }
            Generator::Device(id, version) => {
                id.encode(buffer)?;
                version.encode(buffer)?;
            }
        }
        Ok(())
    }
}
