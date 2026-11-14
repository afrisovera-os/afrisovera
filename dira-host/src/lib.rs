//! Dira's parsers compiled for the host (BROWSER_PLAN.md section 4). Nothing is copied: `#[path]` points at
//! kernel/src/dira, so the code under test is the code the kernel builds.
#[path = "../../kernel/src/dira/url.rs"]
pub mod url;
#[path = "../../kernel/src/dira/http.rs"]
pub mod http;
#[path = "../../kernel/src/dira/html.rs"]
pub mod html;

#[cfg(test)]
mod tests {
    use super::html::{self, Html, Page};
    use super::http::{Kind, Resp};
    use super::url::{self, Shown};

    fn show(u: &url::Url) -> String {
        let mut s = Shown::new();
        url::show(u, &mut s);
        String::from_utf8(s.get().to_vec()).unwrap()
    }

    fn p(s: &str) -> String {
        match url::parse(s.as_bytes()) {
            Ok(u) => show(&u),
            Err(e) => format!("ERR {}", e),
        }
    }

    fn r(base: &str, href: &str) -> String {
        let b = url::parse(base.as_bytes()).unwrap();
        match url::resolve(&b, href.as_bytes()) {
            Ok(Some(u)) => show(&u),
            Ok(None) => "SAME".into(),
            Err(e) => format!("ERR {}", e),
        }
    }

    #[test]
    fn urls() {
        assert_eq!(p("http://Example.COM"), "http://example.com/");
        assert_eq!(p("example.com/a b"), "http://example.com/a%20b");
        assert_eq!(p("  http://10.0.2.2:8000/x?y=1#frag "), "http://10.0.2.2:8000/x?y=1");
        assert_eq!(p("localhost:8080"), "http://localhost:8080/");
        assert_eq!(p("https://a.b/c"), "https://a.b/c");
        assert_eq!(p("http://a.b:80/"), "http://a.b/");
        assert_eq!(p("http://a.b/x/../y/./z"), "http://a.b/y/z");
        assert_eq!(p("http://a.b/../../"), "http://a.b/");
        assert!(p("ftp://x").starts_with("ERR"));
        assert!(p("mailto:me@x").starts_with("ERR"));
        assert!(p("http://user@x").starts_with("ERR"));
        assert!(p("http://x:0/").starts_with("ERR"));
        assert!(p("http://x:65536/").starts_with("ERR"));
        assert!(p("http://[::1]/").starts_with("ERR"));
        assert!(p("http:///x").starts_with("ERR"));
        assert!(p("http://a_b/").starts_with("ERR"));
        assert_eq!(p("http://a.b./"), "http://a.b/");
        assert!(p("http://e../").starts_with("ERR")); // fuzz finding: stayed "e." and re-parsed as "e"
        assert!(p("").starts_with("ERR"));
        let long = format!("http://x/{}", "a".repeat(500));
        assert!(p(&long).starts_with("ERR"));
        assert_eq!(url::ipv4(&url::parse(b"10.0.2.2").unwrap()), Some([10, 0, 2, 2]));
        assert_eq!(url::ipv4(&url::parse(b"10.0.2.256").unwrap()), None);
    }

    #[test]
    fn links() {
        let b = "http://h.x/a/b/c.html?q=1";
        assert_eq!(r(b, "d.html"), "http://h.x/a/b/d.html");
        assert_eq!(r(b, "../d"), "http://h.x/a/d");
        assert_eq!(r(b, "../../../../d"), "http://h.x/d");
        assert_eq!(r(b, "/root"), "http://h.x/root");
        assert_eq!(r(b, "?z=2"), "http://h.x/a/b/c.html?z=2");
        assert_eq!(r(b, "#top"), "SAME");
        assert_eq!(r(b, "//other.y/p"), "http://other.y/p");
        assert_eq!(r(b, "HTTPS://S.y"), "https://s.y/");
        assert_eq!(r(b, "./"), "http://h.x/a/b/");
        assert_eq!(r(b, "e f#g"), "http://h.x/a/b/e%20f");
        assert!(r(b, "javascript:alert(1)").starts_with("ERR"));
        assert!(r(b, "mailto:a@b").starts_with("ERR"));
        assert_eq!(r("http://h.x", "x"), "http://h.x/x");
    }

    struct Got {
        body: Vec<u8>,
        more: bool,
    }

    fn http_in(chunks: &[&[u8]]) -> (Box<Resp>, Got) {
        let mut resp = Box::new(Resp::new());
        let mut g = Got { body: vec![], more: true };
        for c in chunks {
            if !g.more {
                break;
            }
            let mut sink = |_: &Resp, b: &[u8]| g.body.extend_from_slice(b);
            g.more = resp.feed(c, &mut sink);
        }
        (resp, g)
    }

    #[test]
    fn http_length() {
        let (r, g) = http_in(&[b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Le", b"ngth: 5\r\n\r\nhel", b"loEXTRA"]);
        assert_eq!(r.status, 200);
        assert_eq!(r.kind, Kind::Html);
        assert_eq!(g.body, b"hello");
        assert!(r.complete() && !g.more);
    }

    #[test]
    fn http_chunked() {
        let msg: &[u8] = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 999\r\nContent-Type: text/plain\r\n\r\n4\r\nWiki\r\n5;ext=1\r\npedia\r\nE\r\n in\r\n\r\nchunks.\r\n0\r\n\r\n";
        // every split point gives the same body
        for cut in 0..msg.len() {
            let (r, g) = http_in(&[&msg[..cut], &msg[cut..]]);
            assert_eq!(g.body, b"Wikipedia in\r\n\r\nchunks.", "cut {}", cut);
            assert!(r.complete());
            assert_eq!(r.kind, Kind::Text);
        }
    }

    #[test]
    fn http_other() {
        let (r, g) = http_in(&[b"HTTP/1.1 302 Found\r\nLocation: /next\r\nContent-Length: 3\r\n\r\nabc"]);
        assert!(r.redirect() && !g.more && g.body.is_empty());
        assert_eq!(r.location.get(), b"/next");
        let (r, g) = http_in(&[b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\n\r\ngone"]);
        assert_eq!(r.status, 404);
        assert_eq!(r.reason.get(), b"Not Found");
        assert_eq!(g.body, b"gone");
        assert!(g.more); // read to close
        let (r, _) = http_in(&[b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 4\r\n\r\n\x89PNG"]);
        assert_eq!(r.kind, Kind::Other);
        let (r, _) = http_in(&[b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\n\r\n"]);
        assert!(r.error.is_some());
        let (r, _) = http_in(&[b"SSH-2.0-OpenSSH\r\n\r\n"]);
        assert!(r.error.is_some());
        let (r, _) = http_in(&[b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n"]);
        assert!(r.error.is_some());
        let (r, _) = http_in(&[b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n123456789\r\n"]);
        assert!(r.error.is_some());
        let big = vec![b'a'; 5000];
        let mut m = b"HTTP/1.1 200 OK\r\nX: ".to_vec();
        m.extend_from_slice(&big);
        let (r, _) = http_in(&[&m]);
        assert!(r.error.is_some());
    }

    fn render(html_src: &[u8], width: usize) -> (Box<Page>, Vec<String>) {
        let mut page = Box::new(Page::new());
        let mut h = Box::new(Html::new());
        h.start(false, false);
        h.feed(&mut page, html_src);
        h.finish(&mut page);
        let mut starts = vec![0u32; 4096];
        let rows = html::wrap(&page, width, &mut starts);
        let mut out = vec![];
        for k in 0..rows {
            let (a, b) = html::row(&page, &starts, rows, k, width);
            out.push((a..b).map(|i| page.at(i).0 as char).collect());
        }
        (page, out)
    }

    const DOC: &[u8] = br##"<!DOCTYPE html><html><head><title>Dira  test &amp; page</title>
<style>p { color: red } </p></style><script>var x = "<p>not text</p>"; if (a < b) {}</script></head>
<body><h1>Habari</h1><p>Hello   <b>world</b>, a&nbsp;b &lt;tag&gt; &#65;&#x42; caf&eacute; &bogus; AT&T</p>
<!-- a comment with <p>tags</p> -->
<ul><li>one</li><li>two <a href="/two.html">second&amp;link</a></li></ul>
<ol><li>first<li>second</ol>
<pre>
  keep   this
	tab</pre>
<p>Image: <img src=x.png alt="a  sunset"> and <a href="javascript:void(0)">no</a> <a href="#top">top</a>.</p>
<svg><text>hidden</text></svg><p>3 < 4 and 5 > 2</p></body></html>"##;

    #[test]
    fn html_text() {
        let (page, rows) = render(DOC, 80);
        assert_eq!(page.title.get(), b"Dira test & page");
        let want = [
            "Habari",
            "",
            "Hello world, a b <tag> AB cafe &bogus; AT&T",
            "",
            "* one",
            "* two [1]second&link",
            "",
            "1. first",
            "2. second",
            "",
            "  keep   this",
            "    tab",
            "",
            "Image: [a sunset] and no top.",
            "",
            "3 < 4 and 5 > 2",
        ];
        assert_eq!(rows, want);
        assert_eq!(page.nlinks, 1);
        assert_eq!(page.link(1), Some(&b"/two.html"[..]));
        // roles: the heading in gold, the link in the accent role
        assert_eq!(page.at(0), (b'H', html::R_HEAD));
        let k = (0..page.n).find(|&i| page.at(i).0 == b'[').unwrap();
        assert_eq!(page.at(k).1, html::R_LINK);
    }

    #[test]
    fn html_any_split() {
        let (whole, want) = render(DOC, 30);
        for step in [1usize, 2, 3, 7, 64] {
            let mut page = Box::new(Page::new());
            let mut h = Box::new(Html::new());
            h.start(false, false);
            for c in DOC.chunks(step) {
                h.feed(&mut page, c);
            }
            h.finish(&mut page);
            assert_eq!(page.n, whole.n, "step {}", step);
            for i in 0..page.n {
                assert_eq!(page.at(i), whole.at(i), "step {} letter {}", step, i);
            }
        }
        assert!(want.iter().all(|l| l.len() <= 30));
    }

    #[test]
    fn utf8_and_latin1() {
        let (_, rows) = render("Ɛ ŋ é “q” — …".as_bytes(), 80);
        assert_eq!(rows, ["\u{80} \u{80} e \"q\" - ..."]);
        let mut page = Box::new(Page::new());
        let mut h = Box::new(Html::new());
        h.start(false, true);
        h.feed(&mut page, b"caf\xe9 \x93x\x94");
        h.finish(&mut page);
        let s: String = (0..page.n).map(|i| page.at(i).0 as char).collect();
        assert_eq!(s, "cafe \"x\"\n");
    }

    #[test]
    fn cut_and_wrap() {
        let mut big = Vec::new();
        for _ in 0..20000 {
            big.extend_from_slice(b"<p>word word</p>");
        }
        let (page, _) = render(&big, 40);
        assert!(page.cut);
        assert_eq!(page.n, html::LETTERS - html::RESERVE);
        let mut page = page;
        page.note(b"[cut]", html::R_DIM);
        assert_eq!(page.n, html::LETTERS - html::RESERVE + 6);
        let (_, rows) = render(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bb", 10);
        assert_eq!(rows[0], "aaaaaaaaaa");
        assert_eq!(rows.last().unwrap(), "aaaa bb");
    }

    /// Architect's conditions on M1's code: the full paths of the link table and the pool, entities, notes.
    #[test]
    fn conditions() {
        // 300 links: 256 numbered, the rest plain text, and the page says so
        let mut src = Vec::new();
        for k in 0..300 {
            src.extend_from_slice(format!("<a href=\"/p{}\">l{}</a> ", k, k).as_bytes());
        }
        let (page, rows) = render(&src, 80);
        assert_eq!(page.nlinks, 256);
        assert!(page.links_full);
        assert!(rows.join(" ").contains("[256]l255 l256 l257"));
        assert_eq!(page.link(256), Some(&b"/p255"[..]));
        assert_eq!(page.link(257), None);
        // the 24 KiB pool fills before the table: 60 addresses of 500 bytes
        let mut src = Vec::new();
        for k in 0..60 {
            src.extend_from_slice(format!("<a href=\"/{}{}\">x</a>", k, "a".repeat(497)).as_bytes());
        }
        let (page, _) = render(&src, 80);
        assert!(page.links_full);
        assert_eq!(page.nlinks, 49); // 10 of 499 bytes and 39 of 500 fill 24,490 of the 24,576
        // an address longer than 512 bytes is not numbered
        let (page, _) = render(format!("<a href=\"/{}\">x</a>", "b".repeat(600)).as_bytes(), 80);
        assert_eq!(page.nlinks, 0);
        assert!(page.links_full);
        // numeric entities: C1, 0, surrogates, past U+10FFFF and too many digits
        let (_, rows) = render(b"a&#150;b&#0;c&#xD800;d&#x110000;e&#12345678;f&#65", 80);
        assert_eq!(rows, ["a\u{80}b\u{80}c\u{80}d\u{80}e&#12345678;f&#65"]);
        // a note keeps only letters the cells can draw
        let mut page = Box::new(Page::new());
        page.note(b"a\x1b[2Jb\x07\xff", html::R_ERROR);
        let s: Vec<u8> = (0..page.n).map(|i| page.at(i).0).collect();
        assert_eq!(s, b"a\x80[2Jb\x80\x80\n");
    }

    #[test]
    fn private_redirects() {
        let ok = url::redirect_ok;
        assert!(ok([93, 184, 216, 34], [1, 1, 1, 1])); // public to public
        assert!(!ok([93, 184, 216, 34], [192, 168, 1, 1])); // public to the router
        assert!(!ok([93, 184, 216, 34], [10, 0, 0, 1]));
        assert!(!ok([93, 184, 216, 34], [172, 16, 0, 1]));
        assert!(ok([93, 184, 216, 34], [172, 32, 0, 1])); // outside 172.16/12: public
        assert!(!ok([93, 184, 216, 34], [127, 0, 0, 1]));
        assert!(!ok([93, 184, 216, 34], [169, 254, 169, 254]));
        assert!(!ok([93, 184, 216, 34], [0, 0, 0, 0]));
        assert!(ok([10, 0, 2, 2], [10, 0, 2, 2])); // the local network to itself
        assert!(ok([10, 0, 2, 2], [192, 168, 0, 1])); // private to private
        assert!(ok([10, 0, 2, 2], [8, 8, 8, 8])); // private to public
        assert!(!ok([10, 0, 2, 2], [169, 254, 169, 254])); // private to link-local
        assert!(!ok([10, 0, 2, 2], [127, 0, 0, 1])); // private to this machine
        assert!(!ok([93, 184, 216, 34], [100, 64, 0, 1])); // public to a VPN overlay machine (security review)
        assert!(!ok([93, 184, 216, 34], [100, 127, 255, 254]));
        assert!(ok([93, 184, 216, 34], [100, 128, 0, 1])); // outside 100.64/10: public
        assert!(ok([93, 184, 216, 34], [100, 63, 0, 1]));
        assert!(!ok([93, 184, 216, 34], [224, 0, 0, 1])); // multicast
        assert!(!ok([93, 184, 216, 34], [255, 255, 255, 255])); // broadcast
        assert!(!ok([10, 0, 2, 2], [240, 0, 0, 1])); // reserved
        assert!(ok([127, 0, 0, 1], [127, 0, 0, 1])); // this machine to itself stays allowed
        assert!(!ok([127, 0, 0, 1], [0, 0, 0, 1])); // architect: 0/8 is never a target, not even from 127
        assert!(!ok([127, 0, 0, 1], [224, 0, 0, 1])); // nor 224/3
        assert!(!ok([0, 0, 0, 1], [0, 0, 0, 1])); // nor 0/8 to itself
    }

    #[test]
    fn http_limits() {
        let (r, _) = http_in(&[b"HTTP/1.1 200 OK\r\nContent-Length: 4194305\r\n\r\n"]);
        assert!(r.error.is_some());
        let (r, _) = http_in(&[b"HTTP/1.1 200 OK\r\nContent-Length: 99999999999999999999999\r\n\r\n"]);
        assert!(r.error.is_some());
        let (r, _) = http_in(&[b"HTTP/1.1 200 OK\r\nContent-Length: 4194304\r\n\r\n"]);
        assert!(r.error.is_none());
        let (r, _) = http_in(&[b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\n"]);
        assert!(r.error.is_some());
        let (r, _) = http_in(&[b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n400001\r\n"]);
        assert!(r.error.is_some());
        let (r, g) = http_in(&[b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n400000\r\nab"]);
        assert!(r.error.is_none() && g.body == b"ab");
        let long = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2;{}\r\nab\r\n0\r\n\r\n", "x".repeat(70));
        let (r, _) = http_in(&[long.as_bytes()]);
        assert!(r.error.is_some());
        let ok = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2;{}\r\nab\r\n0\r\n\r\n", "x".repeat(50));
        let (r, g) = http_in(&[ok.as_bytes()]);
        assert!(r.error.is_none() && r.complete() && g.body == b"ab");
    }

    /// The dira suite's pages read on the host give exactly the text the suite expects from the kernel.
    #[test]
    fn suite_pages() {
        let cases: [(&[u8], &str); 2] = [
            (include_bytes!("../../scripts/lib/dira-pages/index.html"), include_str!("../../scripts/lib/dira-pages/index.expected")),
            (include_bytes!("../../scripts/lib/dira-pages/about.html"), include_str!("../../scripts/lib/dira-pages/about.expected")),
        ];
        for (src, want) in cases {
            let (_, rows) = render(src, 80);
            let got: String = rows.iter().map(|r| format!("{}
", r)).collect();
            assert_eq!(got, want.replace("
", "
"));
        }
    }

    /// No input may panic or run past a buffer: random bytes, and random cuts of a real page.
    #[test]
    fn fuzz_smoke() {
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let alphabet: &[u8] = b"<>/!-&#;=\"' abcpaxhrefltgt\r\n\t0123456789\xc3\xa9\xe2\x80\x9c\xf0\xff";
        for round in 0..3000 {
            let n = (next() % 600) as usize;
            let mut buf = Vec::with_capacity(n);
            for _ in 0..n {
                let v = next();
                buf.push(if round % 2 == 0 { alphabet[(v % alphabet.len() as u64) as usize] } else { v as u8 });
            }
            let mut page = Box::new(Page::new());
            let mut h = Box::new(Html::new());
            h.start(round % 5 == 0, round % 7 == 0);
            for c in buf.chunks(1 + (next() % 40) as usize) {
                h.feed(&mut page, c);
            }
            h.finish(&mut page);
            let mut starts = vec![0u32; 64];
            let rows = html::wrap(&page, 1 + (next() % 50) as usize, &mut starts);
            for k in 0..rows {
                let _ = html::row(&page, &starts, rows, k, 20);
            }
            let mut resp = Box::new(Resp::new());
            let mut sink = |_: &Resp, _: &[u8]| {};
            let mut hb = b"HTTP/1.1 200 OK\r\n".to_vec();
            hb.extend_from_slice(&buf);
            for c in hb.chunks(1 + (next() % 30) as usize) {
                if !resp.feed(c, &mut sink) {
                    break;
                }
            }
            let _ = url::parse(&buf);
            if let Ok(b) = url::parse(b"http://h/a/b") {
                let _ = url::resolve(&b, &buf);
            }
        }
    }
}
