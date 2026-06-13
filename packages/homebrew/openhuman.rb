# Homebrew formula template — rendered by CI, committed to tinyhumansai/homebrew-openhuman.
# Placeholders replaced by .github/workflows/release-packages.yml before commit.
class Openhuman < Formula
  desc "AI-powered assistant for communities — OpenHuman CLI"
  homepage "https://github.com/tinyhumansai/openhuman"
  version "@VERSION@"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/tinyhumansai/openhuman/releases/download/v@VERSION@/openhuman-core-@VERSION@-aarch64-apple-darwin.tar.gz"
      sha256 "@SHA256_MACOS_ARM64@"
    end
    on_intel do
      url "https://github.com/tinyhumansai/openhuman/releases/download/v@VERSION@/openhuman-core-@VERSION@-x86_64-apple-darwin.tar.gz"
      sha256 "@SHA256_MACOS_X64@"
    end
  end

  on_linux do
    on_arm do
      # ARM64 (aarch64)
      url "https://github.com/tinyhumansai/openhuman/releases/download/v@VERSION@/openhuman-core-@VERSION@-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "@SHA256_LINUX_ARM64@"
    end
    on_intel do
      url "https://github.com/tinyhumansai/openhuman/releases/download/v@VERSION@/openhuman-core-@VERSION@-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "@SHA256_LINUX_X64@"
    end
  end

  def install
    bin.install "openhuman-core"
    mv bin/"openhuman-core", bin/"openhuman"
  end

  test do
    system "#{bin}/openhuman", "--version"
  end
end
