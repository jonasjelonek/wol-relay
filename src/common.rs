
pub const BROADCAST_MAC: [u8; 6] = [ 0xff, 0xff, 0xff, 0xff, 0xff, 0xff ];


pub fn check_wol_payload(payload: &[u8]) -> bool {
    if payload.len() < 102 { return false; }

    let blocks: Vec<&[u8]> = payload.chunks(6).collect();
    if blocks[0] != BROADCAST_MAC {
        return false;
    }

    for i in 2..blocks.len() {
        if blocks[i] != blocks[1] {
            return false;
        }
    }

    true
}