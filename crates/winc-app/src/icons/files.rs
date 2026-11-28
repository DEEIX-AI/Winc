//! 文件和目录的图标：按扩展名区分的文件图标（有的画成字母）、目录、新建文件、文档。

use super::*;

/// 文件图标纸面上的记号，表示文件的大类；具体语言靠颜色区分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileGlyph {
    /// 源代码：一对尖括号。
    Code,
    /// 文档和日志：一张折角的纸，纸上几行字。
    Text,
    /// 配置和数据：「键 值」两行。
    Data,
    /// 脚本：提示符 >_。
    Shell,
    /// 图片：山和太阳。
    Image,
    /// 压缩包：拉链。
    Archive,
    /// 音频：音符。
    Audio,
    /// 视频：圆角框里一个播放用的三角形。
    Video,
    /// 字体：一个 A。
    Font,
    /// 程序、库这些二进制文件：一张折角的纸，纸上两行点。
    Binary,
    /// 锁文件、证书、密钥：一把锁。
    Lock,
    /// JSON：一对花括号 { }。
    Json,
    /// 配置（TOML、YAML、INI、.editorconfig、各种 rc）：一个齿轮，按格式配色。
    Config,
    /// Markdown：一个井号 #（标题）。
    Markdown,
    /// Docker（Dockerfile、compose、.dockerignore）：叠起来的集装箱。
    Docker,
    /// Git（.gitignore、.gitattributes、.gitmodules）：分支图。
    Git,
    /// 许可证：一枚带两条飘带的徽章。
    License,
    /// README：一个 i（说明）。
    Info,
    /// 环境变量、密钥配置（.env）：一把钥匙。
    Key,
    /// 包的清单（package.json、Cargo.toml、go.mod、pyproject.toml）：一个立方体的包裹。
    Package,
    /// 表格（csv、xlsx）：带格线的框。
    Table,
    /// 数据库和 SQL：一个圆柱。
    Database,
    /// 编程语言的缩写，用线条画的两三个字母（TS、ETS、JS、PY、GO、RS、C、C++、C#、KT、RB、SW、
    /// TEX），扫描一列文件时可直接分辨语言；没有缩写的语言画 `Code`。
    Letters(&'static str),
    /// Java：一杯冒着热气的咖啡。
    Java,
    /// 项目文件（`.winc-project` 及其他编辑器的项目文件）：叠放的目录，同 [`folders`]。
    Project,
    /// 未识别的类型：空白的纸。
    Plain,
}

impl FileGlyph {
    /// 全部记号（包括 `file_kind` 用到的每一个缩写）：目录菜单按它预先画好每种图标，漏掉的记号
    /// 在菜单里没有图标。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub const ALL: [FileGlyph; 38] = [
        FileGlyph::Code,
        FileGlyph::Text,
        FileGlyph::Data,
        FileGlyph::Shell,
        FileGlyph::Image,
        FileGlyph::Archive,
        FileGlyph::Audio,
        FileGlyph::Video,
        FileGlyph::Font,
        FileGlyph::Binary,
        FileGlyph::Lock,
        FileGlyph::Json,
        FileGlyph::Config,
        FileGlyph::Markdown,
        FileGlyph::Docker,
        FileGlyph::Git,
        FileGlyph::License,
        FileGlyph::Info,
        FileGlyph::Key,
        FileGlyph::Package,
        FileGlyph::Table,
        FileGlyph::Database,
        FileGlyph::Letters("TS"),
        FileGlyph::Letters("ETS"),
        FileGlyph::Letters("JS"),
        FileGlyph::Letters("PY"),
        FileGlyph::Letters("GO"),
        FileGlyph::Letters("RS"),
        FileGlyph::Letters("C"),
        FileGlyph::Letters("C++"),
        FileGlyph::Letters("C#"),
        FileGlyph::Letters("KT"),
        FileGlyph::Letters("RB"),
        FileGlyph::Letters("SW"),
        FileGlyph::Letters("TEX"),
        FileGlyph::Java,
        FileGlyph::Project,
        FileGlyph::Plain,
    ];
}

/// 绘制目录图标，15×12，左上角在 `(x, y)`：线条风格，笔宽 `STROKE`，与顶栏图标一致。
///
/// 打开时前片向前倾倒。
pub fn folder(frame: &mut Frame<'_>, (x, y): (f32, f32), open: bool, color: Color, clip: Rect) {
    let icon = if open { Icon::FolderOpen } else { Icon::Folder };
    draw(
        frame,
        icon,
        (x + 7.5, y + 6.0),
        FOLDER_WIDTH,
        STROKE,
        color,
        clip,
    );
}

/// 绘制一组目录的图标，15×12，左上角在 `(x, y)`：下面一个圆角矩形，上面两道逐渐变短的横线，
/// 表示叠在一起的多个目录。
pub fn folders(frame: &mut Frame<'_>, (x, y): (f32, f32), color: Color, clip: Rect) {
    draw(
        frame,
        Icon::Folders,
        (x + 7.5, y + 6.0),
        FOLDER_WIDTH,
        STROKE,
        color,
        clip,
    );
}

/// 绘制符号链接的角标，以 `(cx, cy)` 为中心：露出底面 `surface` 的圆片上一个指向右上的箭头。
///
/// `highlight` 是所在行的高亮，铺在圆片上，与行的底色一致。
pub fn link_badge(
    frame: &mut Frame<'_>,
    (cx, cy): (f32, f32),
    (surface, highlight): (Color, Option<Color>),
    color: Color,
    clip: Rect,
) {
    let r = 4.5;
    let disc = Rect::new(cx - r, cy - r, r * 2.0, r * 2.0);
    frame.surface_clipped(disc, r, surface, clip);
    if let Some(highlight) = highlight {
        frame.rect_clipped(disc, r, highlight, clip);
    }
    // 箭头横竖各 4 逻辑像素，在网格上占 14 格。
    draw(
        frame,
        Icon::Symlink,
        (cx, cy),
        fit(4.0, 14.0),
        STROKE,
        color,
        clip,
    );
}

/// 绘制小号目录图标，约 12×10，以 `(cx, cy)` 为中心：合上的 [`folder`] 按宽度缩小，线条和小号
/// 分支图标一样细，用于侧栏小号文字旁。
pub fn folder_small(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    const WIDTH: f32 = 1.25;
    draw(frame, Icon::Folder, center, 12.0, WIDTH, color, clip);
}

/// 绘制小号工作树图标，12×12，以 `(cx, cy)` 为中心：左上、右下各一个圆角方框，由一道折线
/// 相连，表示从同一个仓库另外签出的目录。线条和小号分支图标一样细。
pub fn worktree_small(frame: &mut Frame<'_>, center: (f32, f32), color: Color, clip: Rect) {
    const WIDTH: f32 = 1.25;
    draw(frame, Icon::Worktree, center, 12.0, WIDTH, color, clip);
}

/// 绘制「新建文件」图标，占 12×15 的位置（左上角 `(x, y)`）：和文件树工具栏的 [`add_file`] 是
/// 同一个图标，居中放在这块位置。
///
/// 开始页、空编辑区用它，与工具栏上的「新建文件」保持一致。
pub fn new_file(frame: &mut Frame<'_>, (x, y): (f32, f32), color: Color, clip: Rect) {
    add_file(frame, (x + 6.0, y + 7.5), color, clip);
}

/// 绘制描边的文档图标，大小为 `w`×`h`：右上角折起，其余三个角是小圆角。
///
/// 用于无法预览的文件的占位页。圆角不随大小缩放，因此不用图标集里按比例缩放的 [`Icon::File`]。
pub fn document(
    frame: &mut Frame<'_>,
    (x, y): (f32, f32),
    (w, h): (f32, f32),
    color: Color,
    clip: Rect,
) {
    let (r, fold) = (3.0, w * 0.3);
    let (right, bottom) = (x + w, y + h);
    let outline = [
        Stroke::Arc(x + r, y + r, r, -PI, -FRAC_PI_2),
        Stroke::Line(x + r, y, right - fold, y),
        Stroke::Line(right - fold, y, right, y + fold),
        Stroke::Line(right, y + fold, right, bottom - r),
        Stroke::Arc(right - r, bottom - r, r, 0.0, FRAC_PI_2),
        Stroke::Line(right - r, bottom, x + r, bottom),
        Stroke::Arc(x + r, bottom - r, r, FRAC_PI_2, PI),
        Stroke::Line(x, bottom - r, x, y + r),
        // 折角
        Stroke::Line(right - fold, y, right - fold, y + fold - r),
        Stroke::Arc(right - fold + r, y + fold - r, r, FRAC_PI_2, PI),
        Stroke::Line(right - fold + r, y + fold, right, y + fold),
    ];
    frame.stroke(&outline, STROKE, color, clip);
}

/// 绘制文件图标，占 12×15 的位置：线条风格，只画表示类型的记号，约 11 像素见方，与目录图标
/// 一样大，笔宽 `STROKE`；文档类（`Text`、`Binary`、`Plain`）在记号外加一张折角的纸。
///
/// 记号就是这个图标的全部内容，不套纸的轮廓，否则一列文件里只剩一排相同的灰框，类型反而看不清。
/// 除语言缩写外，记号都来自图标集。
pub fn file(frame: &mut Frame<'_>, (x, y): (f32, f32), glyph: FileGlyph, color: Color, clip: Rect) {
    // 记号居中在 12×15 这块位置里。
    let (cx, cy) = (x + FILE_WIDTH / 2.0, y + 7.5);
    let k = MARK_SCALE;
    // 记号的设计稿是 ±3.5 的格子，在图标集里占 18 格。
    let mark = 7.0 * k;
    // 文档类的纸和侧栏开关一样高（12），一行里的图标上下沿齐平。
    let page = 12.0;
    let (icon, center, extent) = match glyph {
        FileGlyph::Letters(text) => {
            frame.stroke(monogram(text, (cx, cy)).as_slice(), STROKE, color, clip);
            return;
        }
        FileGlyph::Code => (Icon::AngleBrackets, (cx, cy), mark),
        FileGlyph::Text => (Icon::FileText, (cx, cy), page),
        FileGlyph::Data => (Icon::KeyValue, (cx, cy), mark),
        FileGlyph::Shell => (Icon::Prompt, (cx, cy), mark),
        FileGlyph::Image => (Icon::Image, (cx, cy), mark),
        FileGlyph::Archive => (Icon::Zipper, (cx, cy), mark),
        FileGlyph::Audio => (Icon::MusicNote, (cx, cy), mark),
        FileGlyph::Video => (Icon::PlaySquare, (cx, cy), mark),
        FileGlyph::Font => (Icon::Font, (cx, cy), mark),
        FileGlyph::Binary => (Icon::FileBinary, (cx, cy), page),
        FileGlyph::Lock => (Icon::Lock, (cx, cy), fit(6.0 * k, 16.0)),
        FileGlyph::Json => (Icon::Braces, (cx, cy), SYMBOL),
        // 齿顶半径 5.7，在图标集里是 9 格。
        FileGlyph::Config => (Icon::Gear, (cx, cy), fit(5.7, 9.0)),
        FileGlyph::Markdown => (Icon::Hash, (cx, cy), fit(6.5 * k, 16.0)),
        FileGlyph::Docker => (Icon::Containers, (cx, cy), mark),
        // 两个提交相距 4.5，在图标集里是 12 格。
        FileGlyph::Git => (Icon::GitBranch, (cx, cy), fit(4.5 * k, 12.0)),
        FileGlyph::License => (Icon::Award, (cx, cy), mark),
        FileGlyph::Info => (Icon::InfoLetter, (cx, cy), mark),
        FileGlyph::Key => (Icon::Key, (cx, cy), mark),
        FileGlyph::Package => (Icon::Cube, (cx, cy), SYMBOL),
        FileGlyph::Table => (Icon::Table, (cx, cy), mark),
        FileGlyph::Database => (Icon::Database, (cx, cy), mark),
        FileGlyph::Java => (Icon::Coffee, (cx, cy), mark),
        FileGlyph::Project => (Icon::Folders, (cx, cy + 0.4), 13.2),
        FileGlyph::Plain => (Icon::File, (cx, cy), page),
    };
    draw(frame, icon, center, extent, STROKE, color, clip);
}

/// 用线条画的编程语言缩写（`FileGlyph::Letters`），整体居中在 `(cx, cy)`。
///
/// 每个字母在 4×6 的格子里（左上角为原点）用折线画，字母间隔 1.5，整体再放大到约 11 像素宽；
/// 「+」「#」窄一些、矮一些，跟在字母后面。没有字体的粗体可用、界面字体在这个字号下又太细，
/// 所以自己画，笔宽与其他图标相同。
fn monogram(text: &str, (cx, cy): (f32, f32)) -> Strokes<MONOGRAM_STROKES> {
    /// 一个字的折线（每条是一串点）。
    type Paths = &'static [&'static [(f32, f32)]];
    // 一个字的折线和它占的宽度。
    fn glyph(ch: char) -> (Paths, f32) {
        let paths: Paths = match ch {
            'T' => &[&[(0.0, 0.0), (4.0, 0.0)], &[(2.0, 0.0), (2.0, 6.0)]],
            'S' => &[&[
                (4.0, 0.9),
                (3.1, 0.0),
                (0.9, 0.0),
                (0.0, 0.9),
                (0.0, 2.1),
                (0.9, 3.0),
                (3.1, 3.0),
                (4.0, 3.9),
                (4.0, 5.1),
                (3.1, 6.0),
                (0.9, 6.0),
                (0.0, 5.1),
            ]],
            'J' => &[&[(4.0, 0.0), (4.0, 4.8), (2.8, 6.0), (1.2, 6.0), (0.0, 4.8)]],
            'P' => &[&[
                (0.0, 6.0),
                (0.0, 0.0),
                (3.0, 0.0),
                (4.0, 1.0),
                (4.0, 2.0),
                (3.0, 3.0),
                (0.0, 3.0),
            ]],
            'Y' => &[
                &[(0.0, 0.0), (2.0, 3.0), (4.0, 0.0)],
                &[(2.0, 3.0), (2.0, 6.0)],
            ],
            'G' => &[&[
                (4.0, 1.0),
                (3.0, 0.0),
                (1.0, 0.0),
                (0.0, 1.0),
                (0.0, 5.0),
                (1.0, 6.0),
                (3.0, 6.0),
                (4.0, 5.0),
                (4.0, 3.0),
                (2.25, 3.0),
            ]],
            'O' => &[&[
                (1.0, 0.0),
                (3.0, 0.0),
                (4.0, 1.0),
                (4.0, 5.0),
                (3.0, 6.0),
                (1.0, 6.0),
                (0.0, 5.0),
                (0.0, 1.0),
                (1.0, 0.0),
            ]],
            'R' => &[
                &[
                    (0.0, 6.0),
                    (0.0, 0.0),
                    (3.0, 0.0),
                    (4.0, 1.0),
                    (4.0, 2.0),
                    (3.0, 3.0),
                    (0.0, 3.0),
                ],
                &[(2.0, 3.0), (4.0, 6.0)],
            ],
            'C' => &[&[
                (4.0, 1.0),
                (3.0, 0.0),
                (1.0, 0.0),
                (0.0, 1.0),
                (0.0, 5.0),
                (1.0, 6.0),
                (3.0, 6.0),
                (4.0, 5.0),
            ]],
            'K' => &[
                &[(0.0, 0.0), (0.0, 6.0)],
                &[(4.0, 0.0), (0.0, 4.0)],
                &[(1.5, 2.5), (4.0, 6.0)],
            ],
            'B' => &[
                &[
                    (0.0, 3.0),
                    (0.0, 0.0),
                    (2.8, 0.0),
                    (3.7, 0.9),
                    (3.7, 2.1),
                    (2.8, 3.0),
                    (0.0, 3.0),
                ],
                &[
                    (0.0, 3.0),
                    (0.0, 6.0),
                    (3.1, 6.0),
                    (4.0, 5.1),
                    (4.0, 3.9),
                    (3.1, 3.0),
                ],
            ],
            'W' => &[&[(0.0, 0.0), (1.0, 6.0), (2.0, 2.5), (3.0, 6.0), (4.0, 0.0)]],
            'E' => &[
                &[(4.0, 0.0), (0.0, 0.0), (0.0, 6.0), (4.0, 6.0)],
                &[(0.0, 3.0), (3.0, 3.0)],
            ],
            'X' => &[&[(0.0, 0.0), (4.0, 6.0)], &[(4.0, 0.0), (0.0, 6.0)]],
            '+' => &[&[(0.0, 3.0), (2.5, 3.0)], &[(1.25, 1.75), (1.25, 4.25)]],
            '#' => &[
                &[(1.0, 1.25), (0.6, 5.25)],
                &[(2.4, 1.25), (2.0, 5.25)],
                &[(0.0, 2.5), (3.0, 2.5)],
                &[(0.0, 4.0), (3.0, 4.0)],
            ],
            _ => &[],
        };
        let width = match ch {
            '+' => 2.5,
            '#' => 3.0,
            _ => 4.0,
        };
        (paths, width)
    }

    const GAP: f32 = 1.5;
    let count = text.chars().count();
    let width: f32 =
        text.chars().map(|c| glyph(c).1).sum::<f32>() + GAP * count.saturating_sub(1) as f32;
    // 两个字母时放大到约 11 像素宽；三个（C++、ETS、TEX）时宽度一样，字小一点。
    let scale = (11.0 / width).min(1.3);
    let (left, top) = (cx - width * scale / 2.0, cy - 3.0 * scale);
    let mut x = 0.0;
    let mut out = Strokes::new();
    for (paths, w) in text.chars().map(glyph) {
        for path in paths {
            for pair in path.windows(2) {
                let p = |(px, py): (f32, f32)| (left + (x + px) * scale, top + py * scale);
                let (a, b) = (p(pair[0]), p(pair[1]));
                out.push(Stroke::Line(a.0, a.1, b.0, b.1));
            }
        }
        x += w + GAP;
    }
    out
}

/// [`monogram`] 最多的笔画数：「RB」的 18 笔，留些余量。
const MONOGRAM_STROKES: usize = 24;
