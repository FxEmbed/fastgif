# FastGIF

A Rust-based service to quickly convert X/Twitter mp4s to GIFs on the fly

Basically, how this works is:
- You pass along video.twimg.com URLs
- Your video is downloaded and converted into raw yuv4mpegpipe using FFmpeg
- [gifski](https://github.com/ImageOptim/gifski) takes the video piped into it and converts it into a GIF
- The resulting GIF is transferred back to the client as soon as it's done

## Installation (easy mode)

[just use the docker image :D](https://github.com/FxEmbed/fastgif/pkgs/container/fastgif)

## Installation (hard mode)

## Dependencies

- Rust (latest stable version)
- FFmpeg (must be installed and available in PATH)
- gifski (must be installed and available in PATH)

#### FFmpeg

**macOS:**
```bash
brew install ffmpeg
```

**Ubuntu/Debian:**
```bash
sudo apt update
sudo apt install ffmpeg
```

#### gifski

**macOS:**
```bash
brew install gifski
```

**Ubuntu/Debian:**
```bash
cargo install gifski
```

### Build and Run

1. Clone the repository
2. Build the project:
```bash
cargo build --release
```
3. Run the server:
```bash
cargo run --release
```

The server will start on http://localhost:3000

## Usage

To convert a Twitter video to GIF, make a GET request to:

```
http://localhost:3000/{twitter_video_path}
```

For example, if the original Twitter video URL is:
```
https://video.twimg.com/tweet_video/FfyEjQ_WIAAd7rg.mp4
```

You would request:
```
http://localhost:3000/tweet_video/FfyEjQ_WIAAd7rg.mp4
```

The server will respond with a GIF of the video.

## Configuration

The server runs on port 3000 by default. You can customize it using the PORT environment variable.