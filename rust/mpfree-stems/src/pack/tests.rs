use super::*;
use crate::model::Model;
use crate::net::Kind;

/// A LevelDB-format table as TensorFlow writes a bundle's index: the
/// entries (sorted, the header's empty key first) in data blocks of
/// `per` entries, keys prefix-compressed against the one before, an
/// index block, an empty metaindex, the footer.
fn table(entries: &[(Vec<u8>, Vec<u8>)], per: usize) -> Vec<u8> {
    fn var(out: &mut Vec<u8>, mut v: u64) {
        while v >= 0x80 {
            out.push((v as u8 & 0x7f) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }
    fn block(out: &mut Vec<u8>, entries: &[(Vec<u8>, Vec<u8>)]) -> (u64, u64) {
        let start = out.len();
        let mut last: &[u8] = &[];
        for (k, v) in entries {
            let shared = k.iter().zip(last).take_while(|(a, b)| a == b).count();
            var(out, shared as u64);
            var(out, (k.len() - shared) as u64);
            var(out, v.len() as u64);
            out.extend_from_slice(&k[shared..]);
            out.extend_from_slice(v);
            last = k;
        }
        out.extend_from_slice(&0u32.to_le_bytes()); // one restart, at 0
        out.extend_from_slice(&1u32.to_le_bytes());
        let size = out.len() - start;
        out.push(0); // uncompressed
        out.extend_from_slice(&mask(crc32c(&out[start..start + size + 1])).to_le_bytes());
        (start as u64, size as u64)
    }
    let mut out = Vec::new();
    let mut index = Vec::new();
    for chunk in entries.chunks(per) {
        let (off, size) = block(&mut out, chunk);
        let mut h = Vec::new();
        var(&mut h, off);
        var(&mut h, size);
        index.push((chunk.last().unwrap().0.clone(), h));
    }
    let meta = block(&mut out, &[]);
    let idx = block(&mut out, &index);
    let mut footer = Vec::new();
    for v in [meta.0, meta.1, idx.0, idx.1] {
        var(&mut footer, v);
    }
    footer.resize(40, 0);
    footer.extend_from_slice(&MAGIC.to_le_bytes());
    out.extend_from_slice(&footer);
    out
}

/// A BundleEntryProto for a float32 tensor.
fn proto(shape: &[usize], offset: usize, size: usize, crc: u32) -> Vec<u8> {
    fn var(out: &mut Vec<u8>, mut v: u64) {
        while v >= 0x80 {
            out.push((v as u8 & 0x7f) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }
    let mut dims = Vec::new();
    for &d in shape {
        let mut dim = vec![0x08];
        var(&mut dim, d as u64);
        dims.push(0x12);
        var(&mut dims, dim.len() as u64);
        dims.extend_from_slice(&dim);
    }
    let mut out = vec![0x08, 0x01, 0x12];
    var(&mut out, dims.len() as u64);
    out.extend_from_slice(&dims);
    out.push(0x20);
    var(&mut out, offset as u64);
    out.push(0x28);
    var(&mut out, size as u64);
    out.push(0x35);
    out.extend_from_slice(&crc.to_le_bytes());
    out
}

/// A checkpoint of named tensors: (index, data).
fn bundle(tensors: &[(String, Vec<usize>, Vec<f32>)], per: usize) -> (Vec<u8>, Vec<u8>) {
    let mut data = Vec::new();
    let mut entries = vec![(Vec::new(), vec![0x08, 0x01])]; // the header
    let mut named: Vec<_> = tensors.iter().collect();
    named.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, shape, v) in named {
        let off = data.len();
        for x in v {
            data.extend_from_slice(&x.to_le_bytes());
        }
        let crc = mask(crc32c(&data[off..]));
        entries.push((
            name.clone().into_bytes(),
            proto(shape, off, v.len() * 4, crc),
        ));
    }
    (table(&entries, per), data)
}

#[test]
fn crc32c_has_its_check_value() {
    assert_eq!(crc32c(b"123456789"), 0xe306_9283);
}

#[test]
fn the_index_reader_reads_a_tables_tensors_and_checks_their_bytes() {
    let ts = vec![
        (
            "conv2d/kernel".to_string(),
            vec![2, 3],
            vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        ),
        ("conv2d/bias".to_string(), vec![3], vec![-1.0, 0.5, 0.25]),
        ("conv2d_1/kernel".to_string(), vec![1], vec![7.0]),
    ];
    for per in [1, 2, 10] {
        let (index, data) = bundle(&ts, per);
        let b = Bundle::from_bytes(&index, data.clone()).unwrap();
        for (name, shape, v) in &ts {
            let (s, got) = b.tensor(name).unwrap();
            assert_eq!(s, shape.iter().map(|&d| d as i64).collect::<Vec<_>>());
            assert_eq!(&got, v, "{name} ({per} a block)");
        }
        assert!(b
            .tensor("conv2d_2/kernel")
            .unwrap_err()
            .contains("no tensor"));
        let mut bad = data;
        bad[0] ^= 1;
        let b = Bundle::from_bytes(&index, bad).unwrap();
        assert!(b.tensor("conv2d/bias").unwrap_err().contains("CRC"));
    }
    let (mut index, data) = bundle(&ts, 10);
    let n = index.len();
    index[n - 1] ^= 1;
    let refused = Bundle::from_bytes(&index, data);
    assert!(matches!(refused, Err(Error::Model(m)) if m.contains("not a TensorFlow table")));
}

/// A small network's checkpoint, as Spleeter names it, packed: the model
/// read back is the one built from the same weights with the batch norms
/// folded.
#[test]
fn a_checkpoint_packs_to_the_model_of_its_weights() {
    let shape = Shape {
        filters: [2, 3, 4, 5, 6, 7],
        rows: 64,
    };
    let dims = dims(shape);
    let mut seed = 3u32;
    let mut rnd = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    let mut tensors = Vec::new();
    let mut want: Vec<Vec<f32>> = Vec::new();
    for stem in 0..4 {
        let names = tensor_names(stem);
        for (i, (name, bn)) in names.iter().enumerate() {
            let n: usize = dims[i].iter().product();
            match bn {
                None => {
                    let v: Vec<f32> = (0..n).map(|_| rnd() - 0.5).collect();
                    tensors.push((name.clone(), dims[i].clone(), v.clone()));
                    want.push(v);
                }
                Some(true) => {
                    let parts: Vec<Vec<f32>> = (0..4)
                        .map(|_| (0..n).map(|_| rnd() + 0.5).collect())
                        .collect();
                    for (p, v) in ["gamma", "beta", "moving_mean", "moving_variance"]
                        .iter()
                        .zip(&parts)
                    {
                        tensors.push((format!("{name}/{p}"), dims[i].clone(), v.clone()));
                    }
                    let (g, b, m, var) = (&parts[0], &parts[1], &parts[2], &parts[3]);
                    let sc: Vec<f32> = (0..n).map(|k| g[k] / (var[k] + 1e-3).sqrt()).collect();
                    let sh = (0..n).map(|k| b[k] - m[k] * sc[k]).collect();
                    want.push(sc);
                    want.push(sh);
                }
                Some(false) => {}
            }
        }
    }
    let (index, data) = bundle(&tensors, 16);
    let b = Bundle::from_bytes(&index, data).unwrap();
    let mut file = Vec::new();
    let report = pack_bundle(&b, shape, &mut file).unwrap();
    let kernels: usize = shape
        .stem_layout()
        .iter()
        .filter(|(_, k)| matches!(k, Kind::Kernel(_)))
        .map(|(n, _)| n)
        .sum();
    assert_eq!(report.kernels, 4 * kernels as u64);
    let read = Model::read_shaped(&file[..], shape).unwrap();
    let mut it = want.into_iter();
    let built = Model::build(shape, |_, _| crate::net::Values::F32(it.next().unwrap()));
    assert!(read
        .stems
        .iter()
        .zip(&built.stems)
        .all(|(a, b)| a.same_weights(b)));

    // A kernel stored the wrong way round is refused, by name.
    let mut swapped = tensors.clone();
    let k = swapped
        .iter_mut()
        .find(|t| t.0 == "conv2d_1/kernel")
        .unwrap();
    k.1.swap(2, 3);
    let (index, data) = bundle(&swapped, 16);
    let b = Bundle::from_bytes(&index, data).unwrap();
    match pack_bundle(&b, shape, &mut Vec::new()) {
        Err(Error::Model(m)) => assert!(m.contains("conv2d_1/kernel: shape"), "{m}"),
        other => panic!("{other:?}"),
    }
}
