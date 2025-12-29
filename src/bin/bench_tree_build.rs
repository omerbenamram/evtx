#[cfg(feature = "bench")]
mod imp {
    use bumpalo::Bump;
    use evtx::binxml::bench::{TreeBuildCache, build_tree_from_binxml_bytes_in_bump};
    use evtx::{EvtxChunkData, ParserSettings};
    use std::env;
    use std::path::PathBuf;
    use std::sync::Arc;

    const EVTX_FILE_HEADER_SIZE: usize = 4096;
    const EVTX_CHUNK_SIZE: usize = 65536;

    fn parse_args() -> (PathBuf, usize) {
        let mut args = env::args().skip(1);
        let mut path = PathBuf::from("samples/security.evtx");
        let mut loops: usize = 200_000;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--file" => {
                    if let Some(p) = args.next() {
                        path = PathBuf::from(p);
                    }
                }
                "--loops" => {
                    if let Some(v) = args.next() {
                        loops = v.parse().unwrap_or(loops);
                    }
                }
                _ => {}
            }
        }

        (path, loops)
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let (path, loops) = parse_args();

        let file = std::fs::read(&path)?;
        if file.len() < EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_SIZE {
            return Err(format!("file too small: {}", path.display()).into());
        }

        let chunk_bytes =
            file[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_SIZE].to_vec();
        let mut chunk_data = EvtxChunkData::new(chunk_bytes, false)?;
        let settings = Arc::new(ParserSettings::default());
        let mut chunk = chunk_data.parse(Arc::clone(&settings))?;

        let (start, size) = {
            let record = chunk.iter().next().ok_or("no records")??;
            let start = record.binxml_offset as usize;
            let size = record.binxml_size as usize;
            (start, size)
        };

        let mut tree_bump = Bump::new();
        for _ in 0..loops {
            chunk.arena.reset();
            tree_bump.reset();
            let binxml = &chunk.data[start..start + size];
            let mut cache = TreeBuildCache::new(&chunk);
            let root =
                build_tree_from_binxml_bytes_in_bump(binxml, &chunk, &mut cache, &tree_bump)?;
            std::hint::black_box(root);
        }

        Ok(())
    }
}

#[cfg(feature = "bench")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    imp::run()
}

#[cfg(not(feature = "bench"))]
fn main() {
    eprintln!("`bench_tree_build` requires `--features bench`");
}
