//! CutCraft core: document model, geometry, editing operations, file I/O,
//! cut-job generation and cutter communication. UI-free so it can be tested
//! and reused by other front ends.

pub mod corners;
pub mod curvefit;
pub mod cutjob;
pub mod device;
pub mod dieline;
pub mod doc;
pub mod effects;
pub mod effectstack;
pub mod fcm;
pub mod fx;
pub mod geom;
pub mod hpglimport;
pub mod pdfimport;
pub mod pathtools;
pub mod raster;
pub mod script;
pub mod shapebuilder;
pub mod shapelib;
pub mod singleline;
pub mod tiles;
pub mod trim;
pub mod gpgl;
pub mod io;
pub mod library;
pub mod machines;
pub mod materials;
pub mod ops;
pub mod params;
pub mod printflat;
pub mod prepare;
pub mod prefs;
pub mod queue;
pub mod regmarks;
pub mod text;
pub mod trace;
pub mod warp;
pub mod web;
pub mod adblock;
pub mod barcode;
pub mod mailmerge;
pub mod pixscan;
pub mod spell;
pub mod userfont;
pub mod ble;
pub mod brother_usb;
pub mod calibration;

pub use kurbo;
