class Welinker < Formula
  desc "WeChat iLink to AI Agent bridge"
  homepage "https://github.com/Leejaywell/welinker"
  head "https://github.com/Leejaywell/welinker.git", branch: "main"

  depends_on "rust" => :build
  depends_on "node"

  def install
    system "npm", "ci", "--prefix", "web"
    system "cargo", "install", *std_cargo_args(path: ".")
  end

  test do
    assert_match "welinker", shell_output("#{bin}/welinker version")
  end
end
