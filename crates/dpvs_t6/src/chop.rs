use crate::plane::plane_dist;

/// Points within this of a plane count as on it. BO2 value unknown; MW2's.
pub const CHOP_EPSILON: f32 = 0.001;

/// R_ChopPortalWinding: keep the part of the winding in front of `plane`. Returns the new
/// point count, 0 when nothing is left in front.
pub fn chop_winding(input: &[[f32; 3]], plane: [f32; 4], out: &mut [[f32; 3]]) -> usize {
    let n = input.len();
    if n < 3 {
        return 0;
    }
    let mut front = 0usize;
    let mut back = 0usize;
    for &v in input {
        let d = plane_dist(plane, v);
        if d > CHOP_EPSILON {
            front += 1;
        } else if d < -CHOP_EPSILON {
            back += 1;
        }
    }
    if front == 0 {
        return 0;
    }
    if back == 0 {
        let k = n.min(out.len());
        out[..k].copy_from_slice(&input[..k]);
        return k;
    }
    let mut k = 0usize;
    let mut push = |p: [f32; 3], k: &mut usize| {
        if *k < out.len() {
            out[*k] = p;
            *k += 1;
        }
    };
    for i in 0..n {
        let a = input[i];
        let b = input[(i + 1) % n];
        let da = plane_dist(plane, a);
        let db = plane_dist(plane, b);
        if da >= -CHOP_EPSILON {
            push(a, &mut k);
        }
        let crosses = (da > CHOP_EPSILON && db < -CHOP_EPSILON)
            || (da < -CHOP_EPSILON && db > CHOP_EPSILON);
        if crosses {
            let t = da / (da - db);
            push(core::array::from_fn(|j| a[j] + (b[j] - a[j]) * t), &mut k);
        }
    }
    if k < 3 { 0 } else { k }
}

/// R_ChopPortal: the winding chopped by every plane in turn, left in `a`. `b` is scratch.
pub fn chop_portal(
    verts: &[[f32; 3]],
    planes: &[[f32; 4]],
    a: &mut [[f32; 3]],
    b: &mut [[f32; 3]],
) -> usize {
    let mut n = verts.len().min(a.len());
    if n < 3 {
        return 0;
    }
    a[..n].copy_from_slice(&verts[..n]);
    for &plane in planes {
        n = chop_winding(&a[..n], plane, b);
        if n == 0 {
            return 0;
        }
        a[..n].copy_from_slice(&b[..n]);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: [[f32; 3]; 4] = [
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [2.0, 2.0, 0.0],
        [0.0, 2.0, 0.0],
    ];

    #[test]
    fn chop_keeps_the_front_half() {
        let mut out = [[0.0; 3]; 8];
        let n = chop_winding(&SQUARE, [1.0, 0.0, 0.0, -1.0], &mut out);
        assert_eq!(n, 4);
        assert!(out[..n].iter().all(|v| v[0] >= 1.0));
    }

    #[test]
    fn chop_behind_leaves_nothing() {
        let mut out = [[0.0; 3]; 8];
        assert_eq!(chop_winding(&SQUARE, [1.0, 0.0, 0.0, -5.0], &mut out), 0);
    }

    #[test]
    fn chop_portal_applies_every_plane() {
        let mut a = [[0.0; 3]; 16];
        let mut b = [[0.0; 3]; 16];
        let planes = [[1.0, 0.0, 0.0, -1.0], [0.0, 1.0, 0.0, -1.0]];
        let n = chop_portal(&SQUARE, &planes, &mut a, &mut b);
        assert_eq!(n, 4);
        assert!(a[..n].iter().all(|v| v[0] >= 1.0 && v[1] >= 1.0));
    }
}
