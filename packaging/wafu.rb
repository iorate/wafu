# Formula/wafu.rb in iorate/homebrew-tap is generated from packaging/wafu.rb in
# iorate/wafu by the release workflow there, which fills in the version and the
# checksums. Edit packaging/wafu.rb, not the copy in the tap.
#
# This is a formula and not a cask because Homebrew quarantines what a cask
# installs, with no option to turn that off since Homebrew 6, and macOS refuses
# to open a quarantined app that is not notarized.
#
# It installs the prebuilt binary and does not build from source: a tap like
# this has no bottles, so every upgrade would compile for minutes on the user's
# machine, and on Linux the result would use Homebrew's fontconfig, which
# ignores the font preferences of the host in /etc/fonts.
class Wafu < Formula
  desc "Keyboard-driven dual-pane file manager"
  homepage "https://github.com/iorate/wafu"
  version "@VERSION@"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    on_arm do
      url "https://github.com/iorate/wafu/releases/download/v@VERSION@/wafu-v@VERSION@-macos-arm64.tar.gz"
      sha256 "@MACOS_ARM64_SHA256@"
    end
  end

  # No depends_on for Linux: the binary is prebuilt against the host's loader
  # and libraries and would not use Homebrew's.
  on_linux do
    on_intel do
      url "https://github.com/iorate/wafu/releases/download/v@VERSION@/wafu-v@VERSION@-linux-x64.tar.gz"
      sha256 "@LINUX_X64_SHA256@"
    end
  end

  def install
    prefix.install "THIRD-PARTY-LICENSES.txt"
    if OS.mac?
      prefix.install "wafu.app"
      # A script and not a symlink: started through a symlink, the executable
      # is not treated as part of the bundle and the Dock shows neither the
      # icon nor the name of the app. opt_prefix would not keep a Dock icon
      # alive across upgrades either: the Dock records the resolved path.
      bin.write_exec_script prefix/"wafu.app/Contents/MacOS/wafu"
    else
      bin.install "wafu"
      (share/"icons/hicolor/256x256/apps").install "wafu.png"
      (share/"applications/wafu.desktop").write <<~INI
        [Desktop Entry]
        Type=Application
        Name=wafu
        Comment=#{desc}
        Exec=#{opt_bin}/wafu
        Icon=wafu
        Terminal=false
        Categories=System;FileManager;
      INI
    end
  end

  def caveats
    if OS.mac?
      <<~EOS
        wafu.app is installed at:
          #{opt_prefix}/wafu.app
        To have it in /Applications, copy it there, and again after an upgrade:
          cp -R #{opt_prefix}/wafu.app /Applications/
        An icon kept in the Dock from a running wafu points to the directory of
        the installed version and stops working once an upgrade removes it.
      EOS
    else
      <<~EOS
        wafu uses the libraries of the host at run time: fontconfig, xkbcommon,
        Wayland or X11, and EGL or OpenGL.
        For the desktop entry and the icon to show up in the desktop
        environment, add the share directory of Homebrew to XDG_DATA_DIRS in
        the profile of your login shell:
          export XDG_DATA_DIRS="#{HOMEBREW_PREFIX}/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
      EOS
    end
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/wafu --version")
    # Homebrew re-signs a Mach-O file whose library paths it had to rewrite,
    # which breaks the signature of the bundle around it. That does not happen
    # while wafu links only system libraries; this catches the day it does.
    system "codesign", "--verify", "--deep", "--strict", prefix/"wafu.app" if OS.mac?
  end
end
