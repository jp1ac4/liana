use std::convert::TryInto;

pub fn height_u32_from_i32(height: i32) -> u32 {
    height.try_into().expect("height must fit into u32")
}

pub fn height_i32_from_u32(height: u32) -> i32 {
    height.try_into().expect("height must fit into i32")
}

pub fn height_usize_from_i32(height: i32) -> usize {
    height.try_into().expect("height must fit into usize")
}

pub fn height_usize_from_u32(height: u32) -> usize {
    height.try_into().expect("height must fit into usize")
}
