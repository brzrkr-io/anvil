# Homebrew cask for Anvil. Publish to a tap (e.g. brzrkr-io/homebrew-tap) as
# Casks/anvil.rb so `brew install --cask brzrkr-io/tap/anvil` works. Bump
# `version` + `sha256` per release, or keep `sha256 :no_check` and let the in-app
# updater handle upgrades after the first install. The release.yml workflow
# publishes the DMG this points at.
cask "anvil" do
  version "0.1.0"
  sha256 :no_check

  url "https://github.com/brzrkr-io/anvil/releases/download/v#{version}/Anvil_#{version}_aarch64.dmg"
  name "Anvil"
  desc "AI-native macOS console for terminal, editor, git, and DevOps"
  homepage "https://anvil.brzrkr.io"

  depends_on macos: ">= :big_sur"

  app "Anvil.app"

  zap trash: [
    "~/Library/Application Support/com.pjanderson.anvil",
    "~/Library/WebKit/com.pjanderson.anvil",
    "~/Library/Preferences/com.pjanderson.anvil.plist",
    "~/Library/Saved Application State/com.pjanderson.anvil.savedState",
  ]
end
