use nom::combinator::{cond, map};
use nom::Parser;
use nom::multi::count;

use crate::outline::{read_outline, Outline};
use crate::point::{read_point, Point};

#[derive(Debug)]
pub struct PathShape {
    pub start: Point,
    pub outlines: Vec<Outline>,
}

pub fn read_path_shape<'a>(
    outline_count: usize,
) -> impl Parser<&'a [u8], Output = Option<PathShape>, Error = nom::error::Error<&'a [u8]>> {
    cond(
        outline_count > 0,
        map(
            (read_point, count(read_outline, outline_count)),
            move |(start, outlines)| PathShape { start, outlines },
        ),
    )
}
