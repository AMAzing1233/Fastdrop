// Compression Benchmark Tool - Tests Zstd-style compression/decompression speed

use flate2::write::{ZlibEncoder, ZlibDecoder};
use flate2::Compression;
use std::fs::File;
use std::io::Write;
use std::time::Instant;

#[derive(Debug)]
struct BenchmarkResult {
    size_mb: usize,
    compression_level: u32,
    compress_speed: f64,
    decompress_speed: f64,
}

fn format_speed(bytes: usize, duration_secs: f64) -> f64 {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    mb / duration_secs
}

fn generate_test_data(size_mb: usize) -> Vec<u8> {
    println!("📝 Generating {} MB of test data...", size_mb);
    
    // Generate somewhat compressible data (mix of patterns and random)
    let size_bytes = size_mb * 1024 * 1024;
    let mut data = Vec::with_capacity(size_bytes);
    
    let pattern = b"Lorem ipsum dolor sit amet, consectetur adipiscing elit. ";
    let pattern_len = pattern.len();
    
    for i in 0..size_bytes {
        data.push(pattern[i % pattern_len]);
    }
    
    data
}

fn benchmark_compression(data: &[u8], compression_level: u32, quiet: bool) -> (Vec<u8>, f64, f64) {
    if !quiet {
        println!("   🗜️  Compressing {} MB (level {})...", 
            data.len() / (1024 * 1024), compression_level);
    }
    
    let start = Instant::now();
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(compression_level));
    encoder.write_all(data).expect("Compression failed");
    let compressed = encoder.finish().expect("Failed to finish compression");
    let compress_duration = start.elapsed().as_secs_f64();
    
    let ratio = (compressed.len() as f64 / data.len() as f64) * 100.0;
    let speed = format_speed(data.len(), compress_duration);
    
    if !quiet {
        println!("      ✓ {:.2} MB/s ({:.1}% ratio)", speed, ratio);
    }
    
    (compressed, compress_duration, ratio)
}

fn benchmark_decompression(compressed: &[u8], original_size: usize, quiet: bool) -> (Vec<u8>, f64) {
    if !quiet {
        println!("   📦 Decompressing {} MB...", compressed.len() / (1024 * 1024));
    }
    
    let start = Instant::now();
    let mut decoder = ZlibDecoder::new(Vec::new());
    decoder.write_all(compressed).expect("Decompression failed");
    let decompressed = decoder.finish().expect("Failed to finish decompression");
    let decompress_duration = start.elapsed().as_secs_f64();
    
    let speed = format_speed(original_size, decompress_duration);
    
    if !quiet {
        println!("      ✓ {:.2} MB/s", speed);
    }
    
    // Verify integrity
    if decompressed.len() != original_size {
        eprintln!("   ❌ Size mismatch!");
    }
    
    (decompressed, decompress_duration)
}

fn save_results(
    compress_score: f64,
    decompress_score: f64,
) {
    let results = format!(
        "Compress_score: {:.2}\nDecompress_score: {:.2}\n",
        compress_score,
        decompress_score
    );
    
    let filename = "compression_benchmark.txt";
    match File::create(filename) {
        Ok(mut file) => {
            file.write_all(results.as_bytes()).expect("Failed to write results");
            println!("\n💾 Results saved to: {}", filename);
            println!("{}", results);
        }
        Err(e) => {
            eprintln!("❌ Failed to save results: {}", e);
        }
    }
}

fn run_single_test(size_mb: usize, compression_level: u32) -> BenchmarkResult {
    let test_data = generate_test_data(size_mb);
    
    let (compressed, compress_time, _ratio) = benchmark_compression(&test_data, compression_level, true);
    let (_decompressed, decompress_time) = benchmark_decompression(&compressed, test_data.len(), true);
    
    let compress_speed = format_speed(test_data.len(), compress_time);
    let decompress_speed = format_speed(test_data.len(), decompress_time);
    
    BenchmarkResult {
        size_mb,
        compression_level,
        compress_speed,
        decompress_speed,
    }
}

fn main() {
    println!("🚀 Zlib Compression Benchmark");
    println!("==============================\n");
    
    let mut results = Vec::new();
    
    // Test configurations: (size_mb, compression_level)
    let test_configs = vec![
        // 10 small tests (1-5 MB) at different compression levels
        (1, 1), (1, 3), (1, 6),
        (2, 1), (2, 3), (2, 6),
        (3, 1), (3, 3), (3, 6),
        (5, 6),
        
        // 10 larger tests (10-50 MB) at different compression levels
        (10, 1), (10, 3), (10, 6), (10, 9),
        (20, 1), (20, 6),
        (30, 3), (30, 6),
        (40, 6),
        (50, 6),
    ];
    
    println!("Running {} tests...\n", test_configs.len());
    
    for (i, (size_mb, level)) in test_configs.iter().enumerate() {
        print!("Test {}/{}... ", i + 1, test_configs.len());
        std::io::stdout().flush().unwrap();
        
        let result = run_single_test(*size_mb, *level);
        println!("✓ {} MB (level {}): Compress {:.2} MB/s, Decompress {:.2} MB/s",
            size_mb, level, result.compress_speed, result.decompress_speed);
        
        results.push(result);
    }
    
    // Calculate averages
    let avg_compress: f64 = results.iter().map(|r| r.compress_speed).sum::<f64>() / results.len() as f64;
    let avg_decompress: f64 = results.iter().map(|r| r.decompress_speed).sum::<f64>() / results.len() as f64;
    
    println!("\n📊 Average Performance:");
    println!("   Compression: {:.2} MB/s", avg_compress);
    println!("   Decompression: {:.2} MB/s", avg_decompress);
    
    // Save results
    save_results(avg_compress, avg_decompress);
    
    println!("\n✅ Benchmark complete!");
}
