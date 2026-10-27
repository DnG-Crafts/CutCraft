//! Cutter models and their capabilities.

use crate::doc::MatKind;
use crate::gpgl::Protocol;
use crate::gpgl::Protocol::{Dmpl, Fcm, Gpgl, Hpgl};

#[derive(Clone, Debug, PartialEq)]
pub struct Machine {
    pub id: &'static str,
    pub name: &'static str,
    /// Wire language this model speaks (GPGL / HP-GL / DMPL).
    pub protocol: Protocol,
    /// Maximum cutting width, mm.
    pub max_width: f64,
    /// Maximum cutting length on a mat / roll, mm.
    pub max_length: f64,
    pub tool_slots: u8,
    pub mats: &'static [MatKind],
    pub default_mat: MatKind,
    pub bluetooth: bool,
    pub regmarks: bool,
    /// Known USB `(vendor, product)` ids this model enumerates as. Most Silhouette
    /// machines use vendor 0x0B4D; later ones advertise several product ids.
    /// Verify against your own hardware.
    pub usb_ids: &'static [(u16, u16)],
    /// Max force units understood by the firmware.
    pub max_force: u8,
}

pub const GRAPHTEC_VID: u16 = 0x0B4D;

const IN: f64 = 25.4;
const V: u16 = GRAPHTEC_VID;

/// A roll-fed cutter with nominal defaults (width 600 mm, HP-GL unless a
/// protocol is given). Widths are nominal — set the real media size in Page setup.
macro_rules! roll {
    ($id:expr, $name:expr) => { roll!($id, $name, Hpgl) };
    ($id:expr, $name:expr, $proto:expr) => {
        Machine { id: $id, name: $name, protocol: $proto, max_width: 600.0, max_length: 25000.0, tool_slots: 1,
            mats: &[MatKind::None], default_mat: MatKind::None,
            bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 }
    };
}

/// A Brother ScanNCut: driven by saving an `.fcm` file and opening it in
/// CanvasWorkspace (or on the machine) — no direct connection. Cut area
/// measured from CanvasWorkspace's own files (296.67 mm wide).
macro_rules! brother {
    ($id:expr, $name:expr) => {
        Machine { id: $id, name: $name, protocol: Fcm, max_width: 296.67, max_length: 603.6, tool_slots: 1,
            mats: &[MatKind::Mat12x12, MatKind::Mat12x24], default_mat: MatKind::Mat12x12,
            bluetooth: false, regmarks: false, usb_ids: &[], max_force: 33 }
    };
}

pub const MACHINES: &[Machine] = &[
    // --- Silhouette / Graphtec (GPGL) ---
    Machine { id: "cameo5", name: "Cameo 5", protocol: Gpgl, max_width: 12.0 * IN, max_length: 3000.0, tool_slots: 2,
        mats: &[MatKind::Mat12x12, MatKind::Mat12x24, MatKind::None], default_mat: MatKind::Mat12x12,
        bluetooth: true, regmarks: true,
        usb_ids: &[(V, 0x1140), (V, 0x1141), (V, 0x1142), (V, 0x1146), (0x3844, 0x0001)], max_force: 33 },
    Machine { id: "cameo5_plus", name: "Cameo 5 Plus", protocol: Gpgl, max_width: 15.0 * IN, max_length: 3000.0, tool_slots: 2,
        mats: &[MatKind::Mat12x12, MatKind::Mat12x24, MatKind::Mat15x15, MatKind::None], default_mat: MatKind::Mat15x15,
        bluetooth: true, regmarks: true,
        usb_ids: &[(V, 0x1140), (V, 0x1141), (V, 0x1142), (V, 0x1146), (0x3844, 0x0001)], max_force: 33 },
    Machine { id: "cameo4", name: "Cameo 4", protocol: Gpgl, max_width: 12.0 * IN, max_length: 3000.0, tool_slots: 2,
        mats: &[MatKind::Mat12x12, MatKind::Mat12x24, MatKind::None], default_mat: MatKind::Mat12x12,
        bluetooth: true, regmarks: true, usb_ids: &[(V, 0x1137), (V, 0x1138), (V, 0x1139)], max_force: 33 },
    Machine { id: "cameo4_plus", name: "Cameo 4 Plus", protocol: Gpgl, max_width: 15.0 * IN, max_length: 3000.0, tool_slots: 2,
        mats: &[MatKind::Mat12x12, MatKind::Mat12x24, MatKind::Mat15x15, MatKind::None], default_mat: MatKind::Mat15x15,
        bluetooth: true, regmarks: true, usb_ids: &[(V, 0x1137), (V, 0x1138), (V, 0x1139)], max_force: 33 },
    Machine { id: "cameo4_pro", name: "Cameo 4 Pro / Pro Mk2", protocol: Gpgl, max_width: 24.0 * IN, max_length: 3000.0, tool_slots: 2,
        mats: &[MatKind::Mat24x24, MatKind::Mat12x12, MatKind::None], default_mat: MatKind::Mat24x24,
        bluetooth: true, regmarks: true, usb_ids: &[(V, 0x1137), (V, 0x1138), (V, 0x1139)], max_force: 33 },
    Machine { id: "cameo3", name: "Cameo 3", protocol: Gpgl, max_width: 12.0 * IN, max_length: 3000.0, tool_slots: 2,
        mats: &[MatKind::Mat12x12, MatKind::Mat12x24, MatKind::None], default_mat: MatKind::Mat12x12,
        bluetooth: true, regmarks: true, usb_ids: &[(V, 0x112F)], max_force: 33 },
    Machine { id: "portrait4", name: "Portrait 4", protocol: Gpgl, max_width: 8.0 * IN, max_length: 3000.0, tool_slots: 2,
        mats: &[MatKind::Mat8x12, MatKind::None], default_mat: MatKind::Mat8x12,
        bluetooth: true, regmarks: true, usb_ids: &[(V, 0x113F)], max_force: 33 },
    Machine { id: "portrait3", name: "Portrait 3", protocol: Gpgl, max_width: 8.0 * IN, max_length: 3000.0, tool_slots: 1,
        mats: &[MatKind::Mat8x12, MatKind::None], default_mat: MatKind::Mat8x12,
        bluetooth: true, regmarks: true, usb_ids: &[(V, 0x113A)], max_force: 33 },
    Machine { id: "curio2", name: "Curio / Curio 2", protocol: Gpgl, max_width: 11.5 * IN, max_length: 15.0 * IN, tool_slots: 2,
        mats: &[MatKind::Mat12x12], default_mat: MatKind::Mat12x12,
        bluetooth: true, regmarks: true, usb_ids: &[(V, 0x112C), (V, 0x113D)], max_force: 33 },
    Machine { id: "generic", name: "Generic GPGL plotter", protocol: Gpgl, max_width: 12.0 * IN, max_length: 3000.0, tool_slots: 1,
        mats: &[MatKind::Mat12x12, MatKind::Mat12x24, MatKind::None], default_mat: MatKind::Mat12x12,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 33 },

    // --- HP-GL vinyl cutters (widths nominal -- verify for your exact model) ---
    Machine { id: "roland_gx24", name: "Roland CAMM-1 GX-24", protocol: Hpgl, max_width: 584.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
    Machine { id: "roland_gs24", name: "Roland CAMM-1 GS-24", protocol: Hpgl, max_width: 584.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
    Machine { id: "gcc_expert24", name: "GCC Expert II-24", protocol: Hpgl, max_width: 610.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
    Machine { id: "gcc_jaguar", name: "GCC Jaguar V (24\")", protocol: Hpgl, max_width: 610.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
    Machine { id: "gcc_puma", name: "GCC Puma IV (24\")", protocol: Hpgl, max_width: 610.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
    Machine { id: "mutoh_valuecut", name: "Mutoh ValueCut (24\")", protocol: Hpgl, max_width: 610.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
    Machine { id: "uscutter_mh", name: "USCutter MH-721", protocol: Hpgl, max_width: 720.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
    Machine { id: "uscutter_titan", name: "USCutter Titan 3 (28\")", protocol: Hpgl, max_width: 720.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
    Machine { id: "vinylexpress_qe", name: "Vinyl Express Qe6000 (24\")", protocol: Hpgl, max_width: 610.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
    Machine { id: "skycut_c24", name: "SkyCut C24 (24\")", protocol: Hpgl, max_width: 610.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
    Machine { id: "secabo_s60", name: "Secabo S60 (24\")", protocol: Hpgl, max_width: 630.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
    Machine { id: "foison_c24", name: "Foison C24 (24\")", protocol: Hpgl, max_width: 610.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
    Machine { id: "pazzles_vue", name: "Pazzles Inspiration Vue", protocol: Hpgl, max_width: 12.0 * IN, max_length: 3000.0, tool_slots: 1,
        mats: &[MatKind::Mat12x12, MatKind::None], default_mat: MatKind::Mat12x12,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
    // Siser Romeo/Juliet/Momento are SkyCut-based HP-GL cutters (settings class
    // SkyCutSiser) with SkyCut-style registration marks.
    Machine { id: "siser_romeo", name: "Siser Romeo (24\")", protocol: Hpgl, max_width: 600.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: true, usb_ids: &[], max_force: 250 },
    Machine { id: "siser_juliet", name: "Siser Juliet (12\")", protocol: Hpgl, max_width: 300.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: true, usb_ids: &[], max_force: 250 },
    Machine { id: "siser_momento24", name: "Siser Momento (24\")", protocol: Hpgl, max_width: 600.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: true, usb_ids: &[], max_force: 250 },
    Machine { id: "siser_momento12", name: "Siser Momento (12\")", protocol: Hpgl, max_width: 300.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: true, usb_ids: &[], max_force: 250 },
    Machine { id: "generic_hpgl", name: "Generic HP-GL cutter", protocol: Hpgl, max_width: 600.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },

    // --- DMPL cutters (Summa / Houston Instrument / Tenneth) ---
    Machine { id: "summa_s75", name: "Summa S Class (DMPL)", protocol: Dmpl, max_width: 750.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
    Machine { id: "summa_d60", name: "Summa SummaCut D60 (DMPL)", protocol: Dmpl, max_width: 600.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
    Machine { id: "houston_dmp", name: "Houston Instrument DMP (DMPL)", protocol: Dmpl, max_width: 600.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
    Machine { id: "generic_dmpl", name: "Generic DMPL cutter", protocol: Dmpl, max_width: 600.0, max_length: 25000.0, tool_slots: 1,
        mats: &[MatKind::None], default_mat: MatKind::None,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },

    // --- Brother ScanNCut (save an .fcm file for CanvasWorkspace) ---
    brother!("brother_scanncut", "Brother ScanNCut (any model)"),
    brother!("brother_cm300", "Brother ScanNCut CM300"),
    brother!("brother_cm350", "Brother ScanNCut CM350"),
    brother!("brother_cm550dx", "Brother ScanNCut CM550DX"),
    brother!("brother_cm650w", "Brother ScanNCut CM650W"),
    brother!("brother_sdx85", "Brother ScanNCut SDX85"),
    brother!("brother_sdx125", "Brother ScanNCut SDX125"),
    brother!("brother_sdx135", "Brother ScanNCut SDX135"),
    brother!("brother_sdx225", "Brother ScanNCut SDX225"),
    brother!("brother_sdx230d", "Brother ScanNCut SDX230D"),
    brother!("brother_sdx325", "Brother ScanNCut SDX325"),
    brother!("brother_sdx330d", "Brother ScanNCut SDX330D"),
    brother!("brother_sdx1000", "Brother ScanNCut SDX1000"),
    brother!("brother_sdx1200", "Brother ScanNCut SDX1200"),
    brother!("brother_sdx2200d", "Brother ScanNCut SDX2200D"),

    // --- Full brand roster from the SCAL cutter classes (engine by family). ---
    // Proprietary binary machines (Cricut, Janome, KNK/Force/Orbit,
    // BossKut/Zing, Craftwell, SilverBullet) and printer/plugin variants are
    // omitted — their command streams weren't decompiled. All widths nominal.

    // Graphtec (GPGL, same engine as Silhouette)
    roll!("graphtec", "Graphtec (GPGL)", Gpgl),

    // DMPL family (Summa / Houston already above)
    roll!("amcoza", "AmCoZa (DMPL)", Dmpl),
    roll!("tenneth", "Tenneth (DMPL)", Dmpl),
    roll!("tenneth_craft", "Tenneth Craft (DMPL)", Dmpl),

    // HP-GL vinyl cutters
    roll!("anagraph", "AnaGraph"),
    roll!("apd", "APD"),
    roll!("arcsign", "ArcSign"),
    roll!("arthur", "Arthur"),
    roll!("artcreation", "Art Creation"),
    roll!("artsign", "ArtSign"),
    roll!("asc365", "ASC365"),
    roll!("bascocut", "Bascocut"),
    roll!("bcut", "B-Cut"),
    roll!("bdf", "BDF"),
    roll!("blackcat", "Black Cat"),
    roll!("blacksail", "Blacksail"),
    roll!("boyi", "Boyi"),
    roll!("bren", "Bren"),
    roll!("bridge", "Bridge"),
    roll!("calortrans", "Calortrans"),
    roll!("copam", "Copam"),
    roll!("cotek", "Cotek"),
    roll!("croccut", "CrocCut"),
    roll!("cutok", "CutOk"),
    roll!("cuyi", "Cuyi"),
    roll!("dgi", "DGI"),
    roll!("dika", "Dika"),
    roll!("dragon", "Dragon"),
    roll!("eastsign", "EastSign"),
    roll!("ecut", "eCut"),
    roll!("flycut", "FlyCut"),
    roll!("focusbinding", "Focus Binding"),
    roll!("galaxy", "Galaxy"),
    roll!("gcc_icraft", "GCC i-Craft"),
    roll!("gcc_rxii", "GCC RX II"),
    roll!("gerber", "Gerber"),
    roll!("gercutter", "GerCutter"),
    roll!("goldcut", "Goldcut"),
    roll!("gotadetinta", "Gota de Tinta"),
    roll!("hefeikaxing", "Hefei Kaxing"),
    roll!("helo", "Helo"),
    roll!("highlight", "HighLight"),
    roll!("hobbycut", "HobbyCut"),
    roll!("hp", "HP"),
    roll!("ioline", "Ioline"),
    roll!("jiachen", "JiaChen"),
    roll!("jinka", "Jinka"),
    roll!("kasa", "Kasa"),
    roll!("katana", "Katana"),
    roll!("kingcut", "KingCut"),
    roll!("likcut", "LikCut"),
    roll!("liyu", "Liyu"),
    roll!("loklik", "LokLik"),
    roll!("magictransfer", "Magic Transfer"),
    roll!("marsh", "Marsh"),
    roll!("mastervinyl", "Master Vinyl Cutter"),
    roll!("mimaki", "Mimaki"),
    roll!("momo", "MoMo"),
    roll!("monoprice", "Monoprice"),
    roll!("moritzu", "Moritzu"),
    roll!("muse", "Muse"),
    roll!("mycut", "MyCut"),
    roll!("mydigidi", "MyDigiDi"),
    roll!("omnisignplus", "OmniSign Plus"),
    roll!("pcut", "PCut"),
    roll!("pixmax", "PixMax"),
    roll!("polaris", "Polaris"),
    roll!("powercut", "PowerCut"),
    roll!("rabbit", "Rabbit"),
    roll!("ramtin", "Ramtin"),
    roll!("redsail", "Redsail"),
    roll!("refine", "Refine"),
    roll!("rheinstern", "Rheinstern"),
    roll!("rohs", "Rohs"),
    roll!("saga", "Saga"),
    roll!("seiki", "Seiki"),
    roll!("signkey", "SignKey"),
    roll!("signmax", "SignMax"),
    roll!("singer", "Singer Momento"),
    roll!("ssk", "SSK"),
    roll!("starcraft", "StarCraft"),
    roll!("stmrobotics", "STM Robotics"),
    roll!("transmax", "Transmax"),
    roll!("ukcutter", "UKCutter"),
    roll!("vevor", "Vevor"),
    roll!("vevor_smart", "Vevor Smart"),
    roll!("vicsign", "VicSign"),
    roll!("vinylexpress_panther", "Vinyl Express Panther"),
    roll!("vinylexpress_rogue", "Vinyl Express Rogue"),
    roll!("vinyl4", "Vinyl4"),
    roll!("vinylsystems", "Vinyl Systems"),
    roll!("workhorse", "Workhorse"),
    roll!("yinghe", "YingHe"),
    roll!("yow", "Yow"),

    // HP-GL desktop (mat-fed)
    Machine { id: "pazzles", name: "Pazzles Inspiration", protocol: Hpgl, max_width: 12.0 * IN, max_length: 3000.0, tool_slots: 1,
        mats: &[MatKind::Mat12x12, MatKind::None], default_mat: MatKind::Mat12x12,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
    Machine { id: "pazzles_mighty", name: "Pazzles Inspiration Mighty", protocol: Hpgl, max_width: 12.0 * IN, max_length: 3000.0, tool_slots: 1,
        mats: &[MatKind::Mat12x12, MatKind::None], default_mat: MatKind::Mat12x12,
        bluetooth: false, regmarks: false, usb_ids: &[], max_force: 250 },
];

/// Match a Windows USB device-interface path (e.g. `...vid_0b4d&pid_1137...`) to a known
/// machine, using the per-model id map above. Returns the first matching model.
pub fn match_usb_path(path: &str) -> Option<&'static Machine> {
    let lower = path.to_lowercase();
    let hex = |tag: &str| -> Option<u16> {
        let i = lower.find(tag)? + tag.len();
        let end = (i..lower.len()).take_while(|&j| lower.as_bytes()[j].is_ascii_hexdigit()).count();
        u16::from_str_radix(&lower[i..i + end.min(4)], 16).ok()
    };
    let (vid, pid) = (hex("vid_")?, hex("pid_")?);
    MACHINES.iter().find(|m| m.usb_ids.contains(&(vid, pid)))
}

pub fn by_id(id: &str) -> &'static Machine {
    MACHINES.iter().find(|m| m.id == id).unwrap_or(&MACHINES[0])
}
