use x264::{Colorspace, Image, Plane};

pub trait Frame {
    fn as_image(&self) -> Image<'_>;
}

// pub struct EmptyFrame {
//     pub width: usize,
//     pub height: usize,
// }
// 
// impl EmptyFrame {
//     pub fn new(width: usize, height: usize) -> Self {
//         
//     }
// }
// 
// impl Frame for EmptyFrame {
//     fn as_image(&self) -> Image<'_> {
//         Image::new(Colorspace::I420, self.width, self.height)
//     }
// }