//! ECS components that describe compositor entities such as windows, outputs, layers, and seats.

pub mod animation;
pub mod decoration;
pub mod layer;
pub mod output;
pub mod popup;
pub mod seat;
pub mod surface;
pub mod window;
pub mod workspace;
pub mod x11;

pub use animation::*;
pub use decoration::*;
pub use layer::*;
pub use output::*;
pub use popup::*;
pub use seat::*;
pub use surface::*;
pub use window::*;
pub use workspace::*;
pub use x11::*;
