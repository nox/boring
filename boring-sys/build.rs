use std::path::{Path, PathBuf};
use std::process::Command;

// NOTE: this build script is adopted from quiche (https://github.com/cloudflare/quiche)
use std::fs::{self, File};
use std::io;
use std::os::unix::io::AsRawFd;

// Additional parameters for Android build of BoringSSL.
//
// Android NDK < 18 with GCC.
const CMAKE_PARAMS_ANDROID_NDK_OLD_GCC: &[(&str, &[(&str, &str)])] = &[
    (
        "aarch64",
        &[("ANDROID_TOOLCHAIN_NAME", "aarch64-linux-android-4.9")],
    ),
    (
        "arm",
        &[("ANDROID_TOOLCHAIN_NAME", "arm-linux-androideabi-4.9")],
    ),
    (
        "x86",
        &[("ANDROID_TOOLCHAIN_NAME", "x86-linux-android-4.9")],
    ),
    (
        "x86_64",
        &[("ANDROID_TOOLCHAIN_NAME", "x86_64-linux-android-4.9")],
    ),
];

// Android NDK >= 19.
const CMAKE_PARAMS_ANDROID_NDK: &[(&str, &[(&str, &str)])] = &[
    ("aarch64", &[("ANDROID_ABI", "arm64-v8a")]),
    ("arm", &[("ANDROID_ABI", "armeabi-v7a")]),
    ("x86", &[("ANDROID_ABI", "x86")]),
    ("x86_64", &[("ANDROID_ABI", "x86_64")]),
];

const CMAKE_PARAMS_IOS: &[(&str, &[(&str, &str)])] = &[
    (
        "aarch64",
        &[
            ("CMAKE_OSX_ARCHITECTURES", "arm64"),
            ("CMAKE_OSX_SYSROOT", "iphoneos"),
        ],
    ),
    (
        "x86_64",
        &[
            ("CMAKE_OSX_ARCHITECTURES", "x86_64"),
            ("CMAKE_OSX_SYSROOT", "iphonesimulator"),
        ],
    ),
];

/// Returns the platform-specific output path for lib.
///
/// MSVC generator on Windows place static libs in a target sub-folder,
/// so adjust library location based on platform and build target.
/// See issue: https://github.com/alexcrichton/cmake-rs/issues/18
fn get_boringssl_platform_output_path() -> String {
    if cfg!(windows) {
        // Code under this branch should match the logic in cmake-rs
        let debug_env_var = std::env::var("DEBUG").expect("DEBUG variable not defined in env");

        let deb_info = match &debug_env_var[..] {
            "false" => false,
            "true" => true,
            unknown => panic!("Unknown DEBUG={} env var.", unknown),
        };

        let opt_env_var =
            std::env::var("OPT_LEVEL").expect("OPT_LEVEL variable not defined in env");

        let subdir = match &opt_env_var[..] {
            "0" => "Debug",
            "1" | "2" | "3" => {
                if deb_info {
                    "RelWithDebInfo"
                } else {
                    "Release"
                }
            }
            "s" | "z" => "MinSizeRel",
            unknown => panic!("Unknown OPT_LEVEL={} env var.", unknown),
        };

        subdir.to_string()
    } else {
        "".to_string()
    }
}

#[cfg(feature = "fips")]
const BORING_SSL_PATH: &str = "deps/boringssl-fips";
#[cfg(feature = "frankenfips")]
const BORING_SSL_PATH: &str = "deps/boringssl-frankenfips";
#[cfg(not(any(feature = "fips", feature = "frankenfips")))]
const BORING_SSL_PATH: &str = "deps/boringssl";

/// Returns a new cmake::Config for building BoringSSL.
///
/// It will add platform-specific parameters if needed.
fn get_boringssl_cmake_config() -> cmake::Config {
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let pwd = std::env::current_dir().unwrap();

    let mut boringssl_cmake = cmake::Config::new(BORING_SSL_PATH);

    // Add platform-specific parameters.
    match os.as_ref() {
        "android" => {
            let cmake_params_android = if cfg!(feature = "ndk-old-gcc") {
                CMAKE_PARAMS_ANDROID_NDK_OLD_GCC
            } else {
                CMAKE_PARAMS_ANDROID_NDK
            };

            // We need ANDROID_NDK_HOME to be set properly.
            println!("cargo:rerun-if-env-changed=ANDROID_NDK_HOME");
            let android_ndk_home = std::env::var("ANDROID_NDK_HOME")
                .expect("Please set ANDROID_NDK_HOME for Android build");
            let android_ndk_home = std::path::Path::new(&android_ndk_home);
            for (android_arch, params) in cmake_params_android {
                if *android_arch == arch {
                    for (name, value) in *params {
                        eprintln!("android arch={} add {}={}", arch, name, value);
                        boringssl_cmake.define(name, value);
                    }
                }
            }
            let toolchain_file = android_ndk_home.join("build/cmake/android.toolchain.cmake");
            let toolchain_file = toolchain_file.to_str().unwrap();
            eprintln!("android toolchain={}", toolchain_file);
            boringssl_cmake.define("CMAKE_TOOLCHAIN_FILE", toolchain_file);

            // 21 is the minimum level tested. You can give higher value.
            boringssl_cmake.define("ANDROID_NATIVE_API_LEVEL", "21");
            boringssl_cmake.define("ANDROID_STL", "c++_shared");

            boringssl_cmake
        }

        "ios" => {
            for (ios_arch, params) in CMAKE_PARAMS_IOS {
                if *ios_arch == arch {
                    for (name, value) in *params {
                        eprintln!("ios arch={} add {}={}", arch, name, value);
                        boringssl_cmake.define(name, value);
                    }
                }
            }

            // Bitcode is always on.
            let bitcode_cflag = "-fembed-bitcode";

            // Hack for Xcode 10.1.
            let target_cflag = if arch == "x86_64" {
                "-target x86_64-apple-ios-simulator"
            } else {
                ""
            };

            let cflag = format!("{} {}", bitcode_cflag, target_cflag);

            boringssl_cmake.define("CMAKE_ASM_FLAGS", &cflag);
            boringssl_cmake.cflag(&cflag);

            boringssl_cmake
        }

        _ => {
            // Configure BoringSSL for building on 32-bit non-windows platforms.
            if arch == "x86" && os != "windows" {
                let toolchain_file = if cfg!(feature = "fips") {
                    format!("{}/util/32-bit-toolchain.cmake", BORING_SSL_PATH)
                } else {
                    format!("{}/src/util/32-bit-toolchain.cmake", BORING_SSL_PATH)
                };

                boringssl_cmake
                    .define("CMAKE_TOOLCHAIN_FILE", pwd.join(toolchain_file).as_os_str());
            }

            boringssl_cmake
        }
    }
}

fn run_command(command: &mut Command) -> io::Result<()> {
    let exit_status = command.spawn()?.wait()?;

    if !exit_status.success() {
        let err = match exit_status.code() {
            Some(code) => format!("{:?} exited with status: {}", command, code),
            None => format!("{:?} was terminated by signal", command),
        };

        return Err(io::Error::new(io::ErrorKind::Other, err));
    }

    Ok(())
}

fn boring_ssl_path() -> PathBuf {
    std::fs::canonicalize(format!(
        "{}/{}",
        env!("CARGO_MANIFEST_DIR"),
        BORING_SSL_PATH
    ))
    .unwrap()
}

fn ensure_rpk_patch_applied() -> io::Result<()> {
    use libc::{flock, LOCK_EX, LOCK_NB};

    let lock_file = format!("{}/.has_rpk_patch", BORING_SSL_PATH);
    if std::fs::metadata(&lock_file).is_ok() {
        return Ok(());
    }

    let f = File::create(&lock_file).unwrap();
    let status = unsafe { flock(f.as_raw_fd(), LOCK_EX | LOCK_NB) };

    // Don't apply the patch if another process is already applying the patch:
    if status != 0 && io::Error::last_os_error().kind() == io::ErrorKind::WouldBlock {
        return Ok(());
    }

    let src_path =
        std::fs::canonicalize(format!("{}/src", boring_ssl_path().to_str().unwrap())).unwrap();

    let cmd_path = std::fs::canonicalize(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../scripts/apply_rpk_patch.sh"
    ))
    .unwrap();

    let mut cmd = Command::new(cmd_path);
    cmd.current_dir(src_path);
    run_command(&mut cmd)?;

    Ok(())
}

/// Verify that the toolchains match https://csrc.nist.gov/CSRC/media/projects/cryptographic-module-validation-program/documents/security-policies/140sp3678.pdf
/// See "Installation Instructions" under section 12.1.
// TODO: maybe this should also verify the Go and Ninja versions? But those haven't been an issue in practice ...
fn verify_fips_clang_version() -> (&'static str, &'static str) {
    fn version(tool: &str) -> String {
        let output = match Command::new(tool).arg("--version").output() {
            Ok(o) => o,
            Err(e) => {
                eprintln!("warning: missing {}, trying other compilers: {}", tool, e);
                // NOTE: hard-codes that the loop below checks the version
                return String::new();
            }
        };
        assert!(output.status.success());
        let output = std::str::from_utf8(&output.stdout).expect("invalid utf8 output");
        output.lines().next().expect("empty output").to_string()
    }

    const REQUIRED_CLANG_VERSION: &str = "7.0.1";
    for (cc, cxx) in [
        ("clang-7", "clang++-7"),
        ("clang", "clang++"),
        ("cc", "c++"),
    ] {
        let cc_version = version(cc);
        if cc_version.contains(REQUIRED_CLANG_VERSION) {
            assert!(
                version(cxx).contains(REQUIRED_CLANG_VERSION),
                "mismatched versions of cc and c++"
            );
            return (cc, cxx);
        } else if cc == "cc" {
            eprintln!(
                "warning: unsupported clang version \"{}\": FIPS requires clang {}",
                cc_version, REQUIRED_CLANG_VERSION
            );
            return (cc, cxx);
        } else if !cc_version.is_empty() {
            eprintln!(
                "warning: FIPS requires clang version {}, skipping incompatible version \"{}\"",
                REQUIRED_CLANG_VERSION, cc_version
            );
        }
    }
    unreachable!()
}

fn main() -> io::Result<()> {
    use std::env;

    if !cfg!(any(feature = "fips", feature = "frankenfips")) {
        ensure_rpk_patch_applied()?;
    }

    println!("cargo:rerun-if-env-changed=BORING_BSSL_PATH");
    let bssl_dir = std::env::var("BORING_BSSL_PATH").unwrap_or_else(|_| {
        if !Path::new(BORING_SSL_PATH).join("CMakeLists.txt").exists() {
            println!("cargo:warning=fetching boringssl git submodule");
            // fetch the boringssl submodule
            let status = Command::new("git")
                .args(&[
                    "submodule",
                    "update",
                    "--init",
                    "--recursive",
                    BORING_SSL_PATH,
                ])
                .status();
            if !status.map_or(false, |status| status.success()) {
                panic!("failed to fetch submodule - consider running `git submodule update --init --recursive deps/boringssl` yourself");
            }
        }

        let mut cfg = get_boringssl_cmake_config();

        if cfg!(feature = "fuzzing") {
            cfg.cxxflag("-DBORINGSSL_UNSAFE_DETERMINISTIC_MODE")
                .cxxflag("-DBORINGSSL_UNSAFE_FUZZER_MODE");
        }
        if cfg!(feature = "fips") {
            let (clang, clangxx) = verify_fips_clang_version();
            cfg.define("CMAKE_C_COMPILER", clang);
            cfg.define("CMAKE_CXX_COMPILER", clangxx);
            cfg.define("CMAKE_ASM_COMPILER", clang);
            cfg.define("FIPS", "1");
        }

        // no need to use the specific toolchain as for the fips build.
        // only the pre-built bcm.o is relevant for FIPS certification
        // and that is pre-built with the right toolchain (see README for link).
        if cfg!(feature = "frankenfips") {
            cfg.define("FIPS", "1");
        }

        cfg.build_target("ssl").build();
        cfg.build_target("crypto").build().display().to_string()
    });

    let build_path = get_boringssl_platform_output_path();
    if cfg!(any(feature = "fips", feature = "frankenfips")) {
        println!(
            "cargo:rustc-link-search=native={}/build/crypto/{}",
            bssl_dir, build_path
        );
        println!(
            "cargo:rustc-link-search=native={}/build/ssl/{}",
            bssl_dir, build_path
        );
    } else {
        println!(
            "cargo:rustc-link-search=native={}/build/{}",
            bssl_dir, build_path
        );
    }

    // patch <bssl_dir>/libcrypto.a with fips-certified bcm.o
    if cfg!(feature = "frankenfips") {
        let libcrypto_path = format!("{bssl_dir}/libcrypto.a");
        let bcm_o_path = "/opt/boringssl-fips/lib/bcm.o";
        let bcm_o_new_path = format!("{bssl_dir}/build/bcm-fips.o");
        fs::copy(bcm_o_path, &bcm_o_new_path).unwrap();
        // insert fips bcm.o before bcm.c.o into libcrypto.a,
        // so for all duplicate symbols the older bcm.o is used
        run_command(Command::new("ar").args(["rb", "bcm.c.o", &libcrypto_path, &bcm_o_new_path]))
            .expect("failed to run ar command");
    }

    println!("cargo:rustc-link-lib=static=crypto");
    println!("cargo:rustc-link-lib=static=ssl");

    // MacOS: Allow cdylib to link with undefined symbols
    if cfg!(target_os = "macos") {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-undefined,dynamic_lookup");
    }

    println!("cargo:rerun-if-env-changed=BORING_BSSL_INCLUDE_PATH");
    let include_path = std::env::var("BORING_BSSL_INCLUDE_PATH").unwrap_or_else(|_| {
        if cfg!(any(feature = "fips", feature = "frankenfips")) {
            format!("{}/include", BORING_SSL_PATH)
        } else {
            format!("{}/src/include", BORING_SSL_PATH)
        }
    });

    let mut builder = bindgen::Builder::default()
        .derive_copy(true)
        .derive_debug(true)
        .derive_default(true)
        .derive_eq(true)
        .default_enum_style(bindgen::EnumVariation::NewType { is_bitfield: false })
        .default_macro_constant_type(bindgen::MacroTypeVariation::Signed)
        .generate_comments(true)
        .fit_macro_constants(false)
        .size_t_is_usize(true)
        .layout_tests(true)
        .prepend_enum_name(true)
        .rustfmt_bindings(true)
        .clang_args(&["-I", &include_path]);

    let headers = [
        "aes.h",
        "asn1_mac.h",
        "asn1t.h",
        #[cfg(not(feature = "fips"))]
        "blake2.h",
        "blowfish.h",
        "cast.h",
        "chacha.h",
        "cmac.h",
        "cpu.h",
        "curve25519.h",
        "des.h",
        "dtls1.h",
        "hkdf.h",
        "hrss.h",
        "md4.h",
        "md5.h",
        "obj_mac.h",
        "objects.h",
        "opensslv.h",
        "ossl_typ.h",
        "pkcs12.h",
        "poly1305.h",
        "rand.h",
        "rc4.h",
        "ripemd.h",
        "siphash.h",
        "srtp.h",
        #[cfg(not(feature = "fips"))]
        "trust_token.h",
        "x509v3.h",
    ];
    for header in &headers {
        builder = builder.header(
            Path::new(&include_path)
                .join("openssl")
                .join(header)
                .to_str()
                .unwrap(),
        );
    }

    let bindings = builder.generate().expect("Unable to generate bindings");
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");

    Ok(())
}
