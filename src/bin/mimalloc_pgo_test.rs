// Test case to investigate interaction between mimalloc and LLVM PGO
// Compile with:
// RUSTFLAGS="-Cprofile-generate=/tmp/pgo-data" cargo build --release --bin mimalloc_pgo_test --features fast-alloc-mimalloc-secure

// Import mimalloc
#[cfg(feature = "fast-alloc-mimalloc-secure")]
use mimalloc::MiMalloc;

#[cfg(feature = "fast-alloc-mimalloc-secure")]
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

// LLVM profile function definition
unsafe extern "C" {
    fn __llvm_profile_write_file() -> i32;
    
    // Add more profile-related functions for debugging
    fn __llvm_profile_get_num_data() -> u64;
    fn __llvm_profile_get_num_counters() -> u64;
    fn __llvm_profile_get_size_for_buffer() -> u64;
    
    // Profile data pointers
    static __llvm_profile_begin_data: u64;
    static __llvm_profile_end_data: u64;
    static __llvm_profile_begin_counters: u64;
    static __llvm_profile_end_counters: u64;
    static __llvm_profile_filename: *const u8;
}

fn main() {
    println!("=== Starting mimalloc + PGO test ===");
    
    // Create and free various allocations to exercise the allocator
    println!("Performing allocations...");
    let mut allocations = Vec::new();
    
    // Make many allocations of different sizes
    for i in 1..1000 {
        let size = i * 16; // Different sizes
        let data = Box::new(vec![0u8; size]);
        allocations.push(data);
    }
    
    // Print sizes and addresses for debugging
    println!("First allocation address: {:p}", &allocations[0]);
    println!("Last allocation address: {:p}", &allocations[allocations.len()-1]);
    println!("Total allocations: {}", allocations.len());
    
    // Force some deallocations
    println!("Freeing half of allocations...");
    for _ in 0..500 {
        allocations.pop();
    }
    
    // Allocate new memory
    println!("Making more allocations...");
    for i in 0..200 {
        let size = (i % 32) * 128 + 16;
        let data = Box::new(vec![1u8; size]);
        allocations.push(data);
    }
    
    // Print some stats on allocations
    println!("Total allocations now: {}", allocations.len());
    
    // Print info about LLVM profile data
    unsafe {
        println!("Profile data info before writing:");
        println!("  Num data entries: {}", __llvm_profile_get_num_data());
        println!("  Num counters: {}", __llvm_profile_get_num_counters());
        println!("  Size for buffer: {}", __llvm_profile_get_size_for_buffer());
        println!("  Profile data range: {:p} to {:p}", 
                 &__llvm_profile_begin_data as *const _, 
                 &__llvm_profile_end_data as *const _);
        println!("  Profile counters range: {:p} to {:p}", 
                 &__llvm_profile_begin_counters as *const _, 
                 &__llvm_profile_end_counters as *const _);
        
        // Try to print profile filename if available
        if !__llvm_profile_filename.is_null() {
            let filename = std::ffi::CStr::from_ptr(__llvm_profile_filename as *const _)
                .to_string_lossy();
            println!("  Profile filename: {}", filename);
        } else {
            println!("  Profile filename is null");
        }
    }
    
    // Try to write profile data
    println!("\nAttempting to write profile data...");
    unsafe {
        let result = __llvm_profile_write_file();
        println!("Profile data write result: {}", result);
    }
    
    println!("=== Test completed successfully ===");
}