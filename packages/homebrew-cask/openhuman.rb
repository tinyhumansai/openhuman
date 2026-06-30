# Homebrew Cask template - rendered by CI, committed to tinyhumansai/homebrew-openhuman.
# Placeholders are replaced by scripts/release/update-homebrew-cask.sh before commit.
cask "openhuman" do
  version "@VERSION@"
  sha256 arm:   "@SHA256_MACOS_ARM64@",
         intel: "@SHA256_MACOS_X64@"

  on_arm do
    url "https://github.com/tinyhumansai/openhuman/releases/download/v#{version}/OpenHuman_#{version}_aarch64.dmg",
        verified: "github.com/tinyhumansai/openhuman/"
  end

  on_intel do
    url "https://github.com/tinyhumansai/openhuman/releases/download/v#{version}/OpenHuman_#{version}_x64.dmg",
        verified: "github.com/tinyhumansai/openhuman/"
  end

  name "OpenHuman"
  desc "AI-powered personal assistant for communities"
  homepage "https://tinyhumans.ai/openhuman"

  app "OpenHuman.app"

  zap trash: [
    "~/Library/Application Support/com.openhuman.app",
    "~/Library/Caches/com.openhuman.app",
    "~/Library/Logs/com.openhuman.app",
    "~/Library/Preferences/com.openhuman.app.plist",
    "~/.openhuman",
  ]
end
