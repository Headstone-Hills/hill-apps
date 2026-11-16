// macOS text recogniser for Hill Library cover scanning (Apple Vision framework, runs fully on-device).
// Usage: cover-ocr <image>   Prints one line per recognised text line:
//   top<TAB>left<TAB>height<TAB>confidence<TAB>text      (coordinates normalised 0..1, origin top-left)
// Built on first use by server/cover.js (`swiftc -O`); needs the Xcode command line tools.
import Foundation
import Vision
import ImageIO

guard CommandLine.arguments.count > 1,
      let src = CGImageSourceCreateWithURL(URL(fileURLWithPath: CommandLine.arguments[1]) as CFURL, nil),
      let img = CGImageSourceCreateImageAtIndex(src, 0, nil) else {
  FileHandle.standardError.write("cannot read image\n".data(using: .utf8)!)
  exit(2)
}

let req = VNRecognizeTextRequest()
req.recognitionLevel = .accurate
req.usesLanguageCorrection = false      // titles and names are not prose: dictionary "corrections" would damage them
req.recognitionLanguages = ["en-US"]
req.minimumTextHeight = 0.01
do { try VNImageRequestHandler(cgImage: img, options: [:]).perform([req]) } catch {
  FileHandle.standardError.write("vision failed: \(error)\n".data(using: .utf8)!)
  exit(3)
}
for o in (req.results ?? []) {
  guard let c = o.topCandidates(1).first else { continue }
  let b = o.boundingBox
  let text = c.string.replacingOccurrences(of: "\t", with: " ").replacingOccurrences(of: "\n", with: " ")
  print(String(format: "%.4f\t%.4f\t%.4f\t%.3f\t%@", 1 - b.maxY, b.minX, b.height, c.confidence, text))
}
