//! The exact files GoMini needs, pinned by address, size and SHA-256.
//!
//! The runtime is upstream llama.cpp's own Windows release (MIT licensed),
//! the build GoMini was converted and checked with; the model is the owner's
//! Hugging Face repository at a fixed commit. A file that does not match its
//! hash is deleted, never used.

/// One downloadable file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Artifact {
    /// File name on disk.
    pub name: &'static str,
    /// HTTPS address.
    pub url: &'static str,
    /// Lower-case hex SHA-256 of the whole file.
    pub sha256: &'static str,
    /// Size in bytes (for progress and a first sanity check).
    pub size: u64,
}

/// The llama.cpp release whose `llama-server` serves `/v1/systemone`.
pub const LLAMA_BUILD: &str = "b11559";

/// llama.cpp with the Vulkan backend: any GPU with a Vulkan driver
/// (AMD, NVIDIA, Intel).
pub const RUNTIME_VULKAN: Artifact = Artifact {
    name: "llama-b11559-bin-win-vulkan-x64.zip",
    url: "https://github.com/ggml-org/llama.cpp/releases/download/b11559/llama-b11559-bin-win-vulkan-x64.zip",
    sha256: "bf2fb2a7fc5d7669ccabbb6de3f6dd6b54058f213b32e731abc62b590f8566dc",
    size: 32_084_119,
};

/// llama.cpp for the processor only: computers without a usable GPU.
pub const RUNTIME_CPU: Artifact = Artifact {
    name: "llama-b11559-bin-win-cpu-x64.zip",
    url: "https://github.com/ggml-org/llama.cpp/releases/download/b11559/llama-b11559-bin-win-cpu-x64.zip",
    sha256: "1228f133868c02a424bcf663a688e66d8c7d5499077649f305188f756f584377",
    size: 19_501_338,
};

/// Hugging Face commit the model files are taken from.
pub const MODEL_COMMIT: &str = "faa23e1366a356335d846b9e7c8434d94f1bd213";

/// GoMini v6, Q8_0 (the `v6` tag of the model repository; the SHA-256 is
/// what makes it the file, so a changed upload is refused).
pub const MODEL: Artifact = Artifact {
    name: "GoMini-v6-Q8_0.gguf",
    url: "https://huggingface.co/Mhmadhassnz/GoMini/resolve/v6/gguf/GoMini-v6-Q8_0.gguf",
    sha256: "b90ce8410ed8bdb20a282a45ffb5107bea16cbc87a61368a779df31675a49bb6",
    size: 356_866_560,
};

/// The notices that travel with every copy of the model.
pub const MODEL_NOTICES: [Artifact; 3] = [
    Artifact {
        name: "LICENSE",
        url: "https://huggingface.co/Mhmadhassnz/GoMini/resolve/faa23e1366a356335d846b9e7c8434d94f1bd213/LICENSE",
        sha256: "a6cba85bc92e0cff7a450b1d873c0eaa2e9fc96bf472df0247a26bec77bf3ff9",
        size: 10_173,
    },
    Artifact {
        name: "NOTICE",
        url: "https://huggingface.co/Mhmadhassnz/GoMini/resolve/faa23e1366a356335d846b9e7c8434d94f1bd213/NOTICE",
        sha256: "a18e969c89575992381205ac8afd848bd60e3497cd891df2aeaa3a42f9238d07",
        size: 248,
    },
    Artifact {
        name: "THIRD_PARTY_DATA.md",
        url: "https://huggingface.co/Mhmadhassnz/GoMini/resolve/faa23e1366a356335d846b9e7c8434d94f1bd213/THIRD_PARTY_DATA.md",
        sha256: "08bb362090c98290c0ef2672cec816322cd9d179465df0d7f3de1bd6a05ea844",
        size: 1_876,
    },
];

/// Model name reported to agents and in Settings.
pub const MODEL_NAME: &str = "GoMini-v6-Q8_0";
