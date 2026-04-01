/// Dis VM opcode, encoded as a single byte in the binary format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Opcode {
    Nop = 0x00,
    Alt = 0x01,
    Nbalt = 0x02,
    Goto = 0x03,
    Call = 0x04,
    Frame = 0x05,
    Spawn = 0x06,
    Runt = 0x07,
    Load = 0x08,
    Mcall = 0x09,
    Mspawn = 0x0A,
    Mframe = 0x0B,
    Ret = 0x0C,
    Jmp = 0x0D,
    Casew = 0x0E,
    Exit = 0x0F,
    New = 0x10,
    Newa = 0x11,
    Newcb = 0x12,
    Newcw = 0x13,
    Newcf = 0x14,
    Newcp = 0x15,
    Newcm = 0x16,
    Newcmp = 0x17,
    Send = 0x18,
    Recv = 0x19,
    Consb = 0x1A,
    Consw = 0x1B,
    Consp = 0x1C,
    Consf = 0x1D,
    Consm = 0x1E,
    Consmp = 0x1F,
    Headb = 0x20,
    Headw = 0x21,
    Headp = 0x22,
    Headf = 0x23,
    Headm = 0x24,
    Headmp = 0x25,
    Tail = 0x26,
    Lea = 0x27,
    Indx = 0x28,
    Movp = 0x29,
    Movm = 0x2A,
    Movmp = 0x2B,
    Movb = 0x2C,
    Movw = 0x2D,
    Movf = 0x2E,
    Cvtbw = 0x2F,
    Cvtwb = 0x30,
    Cvtfw = 0x31,
    Cvtwf = 0x32,
    Cvtca = 0x33,
    Cvtac = 0x34,
    Cvtwc = 0x35,
    Cvtcw = 0x36,
    Cvtfc = 0x37,
    Cvtcf = 0x38,
    Addb = 0x39,
    Addw = 0x3A,
    Addf = 0x3B,
    Subb = 0x3C,
    Subw = 0x3D,
    Subf = 0x3E,
    Mulb = 0x3F,
    Mulw = 0x40,
    Mulf = 0x41,
    Divb = 0x42,
    Divw = 0x43,
    Divf = 0x44,
    Modw = 0x45,
    Modb = 0x46,
    Andb = 0x47,
    Andw = 0x48,
    Orb = 0x49,
    Orw = 0x4A,
    Xorb = 0x4B,
    Xorw = 0x4C,
    Shlb = 0x4D,
    Shlw = 0x4E,
    Shrb = 0x4F,
    Shrw = 0x50,
    Insc = 0x51,
    Indc = 0x52,
    Addc = 0x53,
    Lenc = 0x54,
    Lena = 0x55,
    Lenl = 0x56,
    Beqb = 0x57,
    Bneb = 0x58,
    Bltb = 0x59,
    Bleb = 0x5A,
    Bgtb = 0x5B,
    Bgeb = 0x5C,
    Beqw = 0x5D,
    Bnew = 0x5E,
    Bltw = 0x5F,
    Blew = 0x60,
    Bgtw = 0x61,
    Bgew = 0x62,
    Beqf = 0x63,
    Bnef = 0x64,
    Bltf = 0x65,
    Blef = 0x66,
    Bgtf = 0x67,
    Bgef = 0x68,
    Beqc = 0x69,
    Bnec = 0x6A,
    Bltc = 0x6B,
    Blec = 0x6C,
    Bgtc = 0x6D,
    Bgec = 0x6E,
    Slicea = 0x6F,
    Slicela = 0x70,
    Slicec = 0x71,
    Indw = 0x72,
    Indf = 0x73,
    Indb = 0x74,
    Negf = 0x75,
    Movl = 0x76,
    Addl = 0x77,
    Subl = 0x78,
    Divl = 0x79,
    Modl = 0x7A,
    Mull = 0x7B,
    Andl = 0x7C,
    Orl = 0x7D,
    Xorl = 0x7E,
    Shll = 0x7F,
    Shrl = 0x80,
    Bnel = 0x81,
    Bltl = 0x82,
    Blel = 0x83,
    Bgtl = 0x84,
    Bgel = 0x85,
    Beql = 0x86,
    Cvtlf = 0x87,
    Cvtfl = 0x88,
    Cvtlw = 0x89,
    Cvtwl = 0x8A,
    Cvtlc = 0x8B,
    Cvtcl = 0x8C,
    Headl = 0x8D,
    Consl = 0x8E,
    Newcl = 0x8F,
    Casec = 0x90,
    Indl = 0x91,
    Movpc = 0x92,
    Tcmp = 0x93,
    Mnewz = 0x94,
    Cvtrf = 0x95,
    Cvtfr = 0x96,
    Cvtws = 0x97,
    Cvtsw = 0x98,
    Lsrw = 0x99,
    Lsrl = 0x9A,
    Eclr = 0x9B,
    Newz = 0x9C,
    Newaz = 0x9D,
    Raise = 0x9E,
    Casel = 0x9F,
    Mulx = 0xA0,
    Divx = 0xA1,
    Cvtxx = 0xA2,
    Mulx0 = 0xA3,
    Divx0 = 0xA4,
    Cvtxx0 = 0xA5,
    Mulx1 = 0xA6,
    Divx1 = 0xA7,
    Cvtxx1 = 0xA8,
    Cvtfx = 0xA9,
    Cvtxf = 0xAA,
    Expw = 0xAB,
    Expl = 0xAC,
    Expf = 0xAD,
    Self_ = 0xAE,
    Brkpt = 0xAF,
}

impl TryFrom<u8> for Opcode {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value <= Opcode::Brkpt as u8 {
            // SAFETY: all values 0x00..=0xAF are defined variants with #[repr(u8)]
            Ok(unsafe { core::mem::transmute::<u8, Opcode>(value) })
        } else {
            Err(value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_opcodes() {
        for byte in 0x00..=0xAF_u8 {
            let op = Opcode::try_from(byte).expect("valid opcode byte");
            assert_eq!(op as u8, byte);
        }
    }

    #[test]
    fn invalid_opcode() {
        assert!(Opcode::try_from(0xB0).is_err());
        assert!(Opcode::try_from(0xFF).is_err());
    }

    #[test]
    fn known_anchor_values() {
        assert_eq!(Opcode::Consmp as u8, 0x1F);
        assert_eq!(Opcode::Mulb as u8, 0x3F);
        assert_eq!(Opcode::Bltw as u8, 0x5F);
        assert_eq!(Opcode::Shll as u8, 0x7F);
        assert_eq!(Opcode::Casel as u8, 0x9F);
    }
}
