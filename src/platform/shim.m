#import <Cocoa/Cocoa.h>
#import <QuartzCore/CAMetalLayer.h>
#import <QuartzCore/CATransaction.h>
#import <QuartzCore/CADisplayLink.h>
#import <Metal/Metal.h>
#import <CoreText/CoreText.h>
#import <CoreGraphics/CoreGraphics.h>
#import <ImageIO/ImageIO.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>
#import <UserNotifications/UserNotifications.h>
#import <unistd.h>

typedef struct {
    uint32_t offset, count;
    float x, y, w, h;
} PaneRange;

typedef struct {
    const void *instances;
    uint32_t count;
    float cell_w, cell_h, pad_x, pad_y;
    float cell_uv[2];
    float bar_h;
    float bg[3];
    float bg_alpha; // clear-color alpha; < 1.0 enables translucency
    float bar_color[3];
    float sep_color[3];
    const float *dividers; // flat x,y,w,h per pane divider
    uint32_t divider_count;
    const float *overlay; // flat x,y,w,h,r,g,b per palette rect
    uint32_t overlay_count;
    uint32_t palette_text_count; // glyph instances after `count`, drawn last
    const void *pending; // PendingGlyph[]: {uint32 cp, uint32 slot, uint32 wide}
    uint32_t pending_count;
    const PaneRange *pane_ranges;
    uint32_t pane_range_count;
} FrameData;

typedef struct {
    float cell[2];
    float viewport[2];
    float cell_uv[2];
} Uniforms;

typedef struct {
    float rect[4];
    float color[4];
    float viewport[2];
} SolidUniforms;

typedef struct {
    uint32_t cols, rows;
    float pt_size;
    float weight;
} AtlasParams;

extern const char *anvil_shader_src(size_t *len);
extern const uint8_t *anvil_font_data(size_t *len);
extern const uint8_t *anvil_icon_data(size_t *len);
extern void anvil_resize(float w, float h);
extern void anvil_frame(FrameData *out);
extern void anvil_atlas_params(AtlasParams *out);
extern void anvil_prewarm_atlas(const void **out_ptr, uint32_t *out_count);
extern void anvil_set_metrics(float cell_w, float cell_h);
extern int anvil_poll(void);
extern void anvil_input(const char *bytes, size_t len);
extern void anvil_paste(const char *bytes, size_t len);
extern void anvil_scroll(int delta);
extern void anvil_mouse(int kind, float x, float y);
extern void anvil_split(int axis);
extern void anvil_close_pane(void);
extern void anvil_focus_dir(int dir);
extern void anvil_new_tab(void);
extern void anvil_cycle_tab(int delta);
extern void anvil_select_tab(int idx);
extern size_t anvil_focused_cwd(char *buf, size_t cap);
extern size_t anvil_window_title(char *buf, size_t cap);
extern void anvil_close_tab(void);
extern void anvil_jump_prompt(int dir);
extern void anvil_resize_pane(int dir);
extern void anvil_balance_panes(void);
extern void anvil_zoom_toggle(void);
extern void anvil_palette_toggle(void);
extern int anvil_palette_open(void);
extern void anvil_palette_char(unsigned char c);
extern void anvil_palette_key(int key);
extern void anvil_search_toggle(void);
extern int anvil_search_open(void);
extern void anvil_search_char(unsigned char c);
extern void anvil_search_key(int key);
extern void anvil_help_toggle(void);
extern int anvil_help_open(void);
extern void anvil_help_key(int key);
extern void anvil_copy_mode_toggle(void);
extern int anvil_copy_mode_open(void);
extern void anvil_copy_mode_key(int key);
extern int anvil_cfg_error_open(void);
extern void anvil_cfg_error_dismiss(void);
extern void anvil_respawn(void);
extern const char *anvil_copy(size_t *out_len);
extern void anvil_caldera_drawer_toggle(void);
extern int anvil_caldera_drawer_open(void);
extern void anvil_caldera_drawer_key(int key);
extern void anvil_set_theme_mode(int mode);
extern void anvil_set_os_dark(int is_dark);
extern int anvil_theme_is_dark(void);
extern void anvil_save_session(void);
extern void anvil_ipc_focus(void);
extern int anvil_link_at(float x, float y, const char **out_ptr, size_t *out_len);
extern bool anvil_needs_render(void);
extern void anvil_force_render(void);

#define INSTANCE_STRIDE (13 * sizeof(float))
#define MAX_INSTANCES 60000
#define ATLAS_SCALE 2.0
#define BAR_H_PT 20.0 // compact title-bar height, logical points

static id<MTLDevice> gDevice;
static id<MTLCommandQueue> gQueue;
static id<MTLRenderPipelineState> gPipeline;
static id<MTLRenderPipelineState> gSolidPipeline;
static id<MTLBuffer> gInstanceBuf;
static id<MTLTexture> gAtlas;
static CTFontRef gFont;   // kept alive for lazy glyph rasterization
static int gGW, gGH;      // glyph cell size in pixels
static uint32_t gCols, gRows; // atlas grid dimensions (cells)
static CGFloat gDescent;  // font descent, for baseline placement
static CGFloat gGlyphStroke; // synthetic-bold weight, raw config 0..2 (0 = off)
static CAMetalLayer *gLayer;
static double gLastW, gLastH;
static NSWindow *gWindow;
static void layoutTrafficLights(NSWindow *win);

static BOOL osIsDark(void) {
    NSString *style = [[NSUserDefaults standardUserDefaults] stringForKey:@"AppleInterfaceStyle"];
    return [style isEqualToString:@"Dark"];
}

// Sync the window's native appearance (traffic lights, vibrancy) to the theme.
static void applyAppearance(void) {
    if (!gWindow) return;
    BOOL dark = anvil_theme_is_dark() != 0;
    gWindow.appearance = [NSAppearance appearanceNamed:dark ? NSAppearanceNameDarkAqua
                                                            : NSAppearanceNameAqua];
    layoutTrafficLights(gWindow);
}

static void buildPipeline(void) {
    size_t len = 0;
    const char *src = anvil_shader_src(&len);
    NSString *code = [[NSString alloc] initWithBytes:src length:len encoding:NSUTF8StringEncoding];

    NSError *err = nil;
    id<MTLLibrary> lib = [gDevice newLibraryWithSource:code options:nil error:&err];
    if (!lib) {
        NSLog(@"shader compile failed: %@", err);
        return;
    }

    MTLRenderPipelineDescriptor *pd = [[MTLRenderPipelineDescriptor alloc] init];
    pd.vertexFunction = [lib newFunctionWithName:@"v_main"];
    pd.fragmentFunction = [lib newFunctionWithName:@"f_main"];
    pd.colorAttachments[0].pixelFormat = gLayer.pixelFormat;

    gPipeline = [gDevice newRenderPipelineStateWithDescriptor:pd error:&err];
    if (!gPipeline) NSLog(@"pipeline failed: %@", err);

    MTLRenderPipelineDescriptor *sd = [[MTLRenderPipelineDescriptor alloc] init];
    sd.vertexFunction = [lib newFunctionWithName:@"v_solid"];
    sd.fragmentFunction = [lib newFunctionWithName:@"f_solid"];
    sd.colorAttachments[0].pixelFormat = gLayer.pixelFormat;
    gSolidPipeline = [gDevice newRenderPipelineStateWithDescriptor:sd error:&err];
    if (!gSolidPipeline) NSLog(@"solid pipeline failed: %@", err);

    gInstanceBuf = [gDevice newBufferWithLength:MAX_INSTANCES * INSTANCE_STRIDE
                                        options:MTLResourceStorageModeShared];
}

// Allocate an empty cols x rows glyph-cache texture (R8). Glyphs are
// rasterized lazily by rasterizeGlyph as Zig requests slots. Grid layout is
// decided by Zig; here is the CoreText/Metal ceremony.
static void buildAtlas(void) {
    AtlasParams ap = {0};
    anvil_atlas_params(&ap);
    gCols = ap.cols;
    gRows = ap.rows;

    CGFloat sz = ap.pt_size * ATLAS_SCALE;
    gGlyphStroke = (CGFloat)ap.weight;
    // Primary font is the bundled Blex Mono Nerd Font (embedded by Zig), so
    // icon/powerline glyphs render directly. System cascade still fills gaps.
    gFont = NULL;
    size_t flen = 0;
    const uint8_t *fdata = anvil_font_data(&flen);
    if (fdata && flen > 0) {
        CFDataRef cfd = CFDataCreateWithBytesNoCopy(NULL, fdata, flen, kCFAllocatorNull);
        CGDataProviderRef prov = CGDataProviderCreateWithCFData(cfd);
        CGFontRef cgf = prov ? CGFontCreateWithDataProvider(prov) : NULL;
        if (cgf) {
            gFont = CTFontCreateWithGraphicsFont(cgf, sz, NULL, NULL);
            CGFontRelease(cgf);
        }
        if (prov) CGDataProviderRelease(prov);
        if (cfd) CFRelease(cfd);
    }
    if (!gFont) gFont = CTFontCreateWithName(CFSTR("Menlo"), sz, NULL);
    CGFloat ascent = CTFontGetAscent(gFont);
    CGFloat descent = CTFontGetDescent(gFont);
    CGFloat leading = CTFontGetLeading(gFont);

    UniChar mch = 'M';
    CGGlyph mg;
    CTFontGetGlyphsForCharacters(gFont, &mch, &mg, 1);
    CGSize adv;
    CTFontGetAdvancesForGlyphs(gFont, kCTFontOrientationHorizontal, &mg, &adv, 1);

    gGW = (int)ceil(adv.width);
    gGH = (int)ceil(ascent + descent + leading);
    gDescent = descent;
    anvil_set_metrics((float)gGW, (float)gGH);

    int aw = gGW * (int)gCols;
    int ah = gGH * (int)gRows;
    MTLTextureDescriptor *td =
        [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatR8Unorm
                                                           width:aw
                                                          height:ah
                                                       mipmapped:NO];
    gAtlas = [gDevice newTextureWithDescriptor:td];
    // Metal textures aren't guaranteed zeroed; clear so unrasterized slots are
    // blank rather than garbage.
    uint8_t *zero = calloc((size_t)aw * ah, 1);
    [gAtlas replaceRegion:MTLRegionMake2D(0, 0, aw, ah)
              mipmapLevel:0
                withBytes:zero
              bytesPerRow:aw];
    free(zero);
}

// Rasterize one codepoint into its cache slot. Uses a CTLine so the system
// font cascade fills glyphs Menlo lacks (box-drawing, symbols, etc).
static void rasterizeGlyph(uint32_t cp, uint32_t slot, uint32_t wide) {
    if (!gAtlas || !gFont) return;
    int col = (int)(slot % gCols);
    int row = (int)(slot / gCols);
    int w = wide ? gGW * 2 : gGW; // wide glyphs span two adjacent cells

    uint8_t *buf = calloc((size_t)w * gGH, 1);
    CGColorSpaceRef gray = CGColorSpaceCreateDeviceGray();
    CGContextRef ctx = CGBitmapContextCreate(buf, w, gGH, 8, w, gray, kCGImageAlphaNone);
    CGContextSetGrayFillColor(ctx, 1.0, 1.0);
    CGContextSetGrayStrokeColor(ctx, 1.0, 1.0); // stroke shares the fill color

    CFStringRef s = CFStringCreateWithBytes(NULL, (const UInt8 *)&cp, 4,
                                            kCFStringEncodingUTF32LE, false);
    if (s) {
        // Draw with the context fill color (white) so the R8 mask captures
        // coverage; without this CTLine defaults to black and the slot stays 0.
        CFStringRef keys[3] = {kCTFontAttributeName, kCTForegroundColorFromContextAttributeName,
                               kCTStrokeWidthAttributeName};
        CFTypeRef vals[3] = {gFont, kCFBooleanTrue, NULL};
        CFIndex nattr = 2;
        // Synthetic bold: a negative stroke width (percent of em) makes CoreText
        // fill AND stroke the glyph, widening every stem. weight 1.0 -> -5%.
        CFNumberRef sw = NULL;
        if (gGlyphStroke > 0.0) {
            double pct = -5.0 * (double)gGlyphStroke;
            sw = CFNumberCreate(NULL, kCFNumberDoubleType, &pct);
            vals[2] = sw;
            nattr = 3;
        }
        CFDictionaryRef attrs = CFDictionaryCreate(NULL, (const void **)keys, (const void **)vals, nattr,
                                                   &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
        CFAttributedStringRef as = CFAttributedStringCreate(NULL, s, attrs);
        CTLineRef line = CTLineCreateWithAttributedString(as);
        CGContextSetTextPosition(ctx, 0, gDescent);
        CTLineDraw(line, ctx);
        CFRelease(line);
        CFRelease(as);
        CFRelease(attrs);
        if (sw) CFRelease(sw);
        CFRelease(s);
    }

    [gAtlas replaceRegion:MTLRegionMake2D(col * gGW, row * gGH, w, gGH)
              mipmapLevel:0
                withBytes:buf
              bytesPerRow:w];

    CGContextRelease(ctx);
    CGColorSpaceRelease(gray);
    free(buf);
}

// Drain the frame's pending-glyph list into the cache texture.
static void drainPending(const FrameData *fd) {
    const uint32_t *pg = (const uint32_t *)fd->pending;
    for (uint32_t i = 0; i < fd->pending_count; i++) {
        rasterizeGlyph(pg[i * 3], pg[i * 3 + 1], pg[i * 3 + 2]);
    }
}

static void drawSolid(id<MTLRenderCommandEncoder> enc, CGSize ds,
                      float x, float y, float w, float h,
                      float r, float g, float b, float a) {
    SolidUniforms su = {
        .rect = {x, y, w, h},
        .color = {r, g, b, a},
        .viewport = {(float)ds.width, (float)ds.height},
    };
    [enc setRenderPipelineState:gSolidPipeline];
    [enc setVertexBytes:&su length:sizeof(su) atIndex:0];
    [enc setFragmentBytes:&su length:sizeof(su) atIndex:0];
    [enc drawPrimitives:MTLPrimitiveTypeTriangleStrip vertexStart:0 vertexCount:4];
}

static void render(void) {
    CGSize ds = gLayer.drawableSize;
    if (ds.width <= 0 || ds.height <= 0) return;

    if (ds.width != gLastW || ds.height != gLastH) {
        gLastW = ds.width;
        gLastH = ds.height;
        anvil_resize((float)ds.width, (float)ds.height);
    }

    if (!anvil_poll()) {
        [NSApp terminate:nil];
        return;
    }

    // Reflect the active tab's label as the window title (Mission Control,
    // Window menu, Cmd+`). Only assign on change to avoid per-frame churn.
    char tbuf[256];
    size_t tn = anvil_window_title(tbuf, sizeof(tbuf));
    if (tn > 0 && gWindow) {
        NSString *title = [[NSString alloc] initWithBytes:tbuf length:tn encoding:NSUTF8StringEncoding];
        if (title && ![gWindow.title isEqualToString:title]) gWindow.title = title;
    }

    if (!anvil_needs_render()) return;

    FrameData fd = {0};
    anvil_frame(&fd);
    drainPending(&fd);

    id<CAMetalDrawable> drawable = [gLayer nextDrawable];
    if (!drawable) return;

    MTLRenderPassDescriptor *rp = [MTLRenderPassDescriptor renderPassDescriptor];
    rp.colorAttachments[0].texture = drawable.texture;
    rp.colorAttachments[0].loadAction = MTLLoadActionClear;
    rp.colorAttachments[0].storeAction = MTLStoreActionStore;
    rp.colorAttachments[0].clearColor = MTLClearColorMake(fd.bg[0], fd.bg[1], fd.bg[2], fd.bg_alpha);

    id<MTLCommandBuffer> cb = [gQueue commandBuffer];
    id<MTLRenderCommandEncoder> enc = [cb renderCommandEncoderWithDescriptor:rp];

    // Title bar first, so tab-label cells (which sit inside the bar) overlay it.
    if (gSolidPipeline && fd.bar_h > 0) {
        drawSolid(enc, ds, 0, 0, (float)ds.width, fd.bar_h,
                  fd.bar_color[0], fd.bar_color[1], fd.bar_color[2], 1.0f);
        drawSolid(enc, ds, 0, fd.bar_h - 1, (float)ds.width, 1,
                  fd.sep_color[0], fd.sep_color[1], fd.sep_color[2], 1.0f);
    }

    Uniforms u = {
        .cell = {fd.cell_w, fd.cell_h},
        .viewport = {(float)ds.width, (float)ds.height},
        .cell_uv = {fd.cell_uv[0], fd.cell_uv[1]},
    };
    uint32_t total = fd.count + fd.palette_text_count;
    if (total > MAX_INSTANCES) total = MAX_INSTANCES;
    if (gPipeline && total > 0) {
        memcpy(gInstanceBuf.contents, fd.instances, total * INSTANCE_STRIDE);
        uint32_t c1 = fd.count > total ? total : fd.count;
        if (c1 > 0) {
            [enc setRenderPipelineState:gPipeline];
            [enc setVertexBytes:&u length:sizeof(u) atIndex:1];
            [enc setFragmentTexture:gAtlas atIndex:0];
            if (fd.pane_range_count > 0) {
                for (uint32_t pi = 0; pi < fd.pane_range_count; pi++) {
                    const PaneRange *pr = &fd.pane_ranges[pi];
                    uint32_t end = pr->offset + pr->count;
                    if (end > c1) end = c1;
                    if (pr->offset >= end) continue;
                    NSUInteger sx = (NSUInteger)pr->x;
                    NSUInteger sy = (NSUInteger)pr->y;
                    NSUInteger sw = (NSUInteger)pr->w;
                    NSUInteger sh = (NSUInteger)pr->h;
                    if (sw == 0 || sh == 0) continue;
                    // Clamp to drawable bounds to avoid Metal validation error.
                    if (sx + sw > (NSUInteger)ds.width)
                        sw = (NSUInteger)ds.width > sx ? (NSUInteger)ds.width - sx : 0;
                    if (sy + sh > (NSUInteger)ds.height)
                        sh = (NSUInteger)ds.height > sy ? (NSUInteger)ds.height - sy : 0;
                    if (sw == 0 || sh == 0) continue;
                    MTLScissorRect scissor = {sx, sy, sw, sh};
                    [enc setScissorRect:scissor];
                    [enc setVertexBuffer:gInstanceBuf
                                  offset:pr->offset * INSTANCE_STRIDE
                                 atIndex:0];
                    [enc drawPrimitives:MTLPrimitiveTypeTriangleStrip
                            vertexStart:0
                            vertexCount:4
                          instanceCount:end - pr->offset];
                }
                // Reset scissor to full drawable for subsequent passes.
                MTLScissorRect full = {0, 0, (NSUInteger)ds.width, (NSUInteger)ds.height};
                [enc setScissorRect:full];
            } else {
                [enc setVertexBuffer:gInstanceBuf offset:0 atIndex:0];
                [enc drawPrimitives:MTLPrimitiveTypeTriangleStrip
                        vertexStart:0
                        vertexCount:4
                      instanceCount:c1];
            }
        }
    }

    if (gSolidPipeline) {
        // Draw hairline dividers (1 logical pt = 2 device px) centered in the gap.
        // The gap rect (d[0..3]) is the hit/layout zone; the drawn line is narrower.
        for (uint32_t i = 0; i < fd.divider_count; i++) {
            const float *d = fd.dividers + i * 4;
            const float kDrawPx = 2.0f; // 1 logical pt @ 2x Retina
            float x = d[0], y = d[1], w = d[2], h = d[3];
            if (w <= h) { // vertical divider: center the hairline horizontally
                x += (w - kDrawPx) * 0.5f;
                w = kDrawPx;
            } else { // horizontal divider: center the hairline vertically
                y += (h - kDrawPx) * 0.5f;
                h = kDrawPx;
            }
            drawSolid(enc, ds, x, y, w, h,
                      fd.sep_color[0], fd.sep_color[1], fd.sep_color[2], 1.0f);
        }
        // Command-palette panel/highlight rects, over the terminal.
        for (uint32_t i = 0; i < fd.overlay_count; i++) {
            const float *o = fd.overlay + i * 7;
            drawSolid(enc, ds, o[0], o[1], o[2], o[3], o[4], o[5], o[6], 1.0f);
        }
    }
    // Palette text last, on top of its panel.
    if (gPipeline && fd.palette_text_count > 0 && fd.count < total) {
        [enc setRenderPipelineState:gPipeline];
        [enc setVertexBuffer:gInstanceBuf offset:fd.count * INSTANCE_STRIDE atIndex:0];
        [enc setVertexBytes:&u length:sizeof(u) atIndex:1];
        [enc setFragmentTexture:gAtlas atIndex:0];
        [enc drawPrimitives:MTLPrimitiveTypeTriangleStrip
                vertexStart:0
                vertexCount:4
              instanceCount:(total - fd.count)];
    }

    [enc endEncoding];
    [cb presentDrawable:drawable];
    [cb commit];
}

// Headless one-shot: render a single frame to a PNG. No window, no run loop.
// For visual verification without Screen Recording permission.
void anvil_dump(const char *path, uint32_t w, uint32_t h) {
    @autoreleasepool {
        gDevice = MTLCreateSystemDefaultDevice();
        gQueue = [gDevice newCommandQueue];
        gLayer = [CAMetalLayer layer];
        gLayer.device = gDevice;
        gLayer.pixelFormat = MTLPixelFormatBGRA8Unorm;

        buildPipeline();
        buildAtlas();
        anvil_resize((float)w, (float)h);

        for (int i = 0; i < 40; i++) {
            if (!anvil_poll()) break;
            usleep(20000);
        }

        FrameData fd = {0};
        anvil_frame(&fd);
        drainPending(&fd);

        MTLTextureDescriptor *td =
            [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatBGRA8Unorm
                                                               width:w height:h mipmapped:NO];
        td.usage = MTLTextureUsageRenderTarget;
        td.storageMode = MTLStorageModeManaged;
        id<MTLTexture> tex = [gDevice newTextureWithDescriptor:td];

        MTLRenderPassDescriptor *rp = [MTLRenderPassDescriptor renderPassDescriptor];
        rp.colorAttachments[0].texture = tex;
        rp.colorAttachments[0].loadAction = MTLLoadActionClear;
        rp.colorAttachments[0].storeAction = MTLStoreActionStore;
        rp.colorAttachments[0].clearColor = MTLClearColorMake(fd.bg[0], fd.bg[1], fd.bg[2], 1.0);

        id<MTLCommandBuffer> cb = [gQueue commandBuffer];
        id<MTLRenderCommandEncoder> enc = [cb renderCommandEncoderWithDescriptor:rp];
        if (gSolidPipeline && fd.bar_h > 0) {
            drawSolid(enc, CGSizeMake(w, h), 0, 0, (float)w, fd.bar_h,
                      fd.bar_color[0], fd.bar_color[1], fd.bar_color[2], 1.0f);
            drawSolid(enc, CGSizeMake(w, h), 0, fd.bar_h - 1, (float)w, 1,
                      fd.sep_color[0], fd.sep_color[1], fd.sep_color[2], 1.0f);
        }
        Uniforms u = {
            .cell = {fd.cell_w, fd.cell_h},
            .viewport = {(float)w, (float)h},
            .cell_uv = {fd.cell_uv[0], fd.cell_uv[1]},
        };
        uint32_t total = fd.count + fd.palette_text_count;
        if (total > MAX_INSTANCES) total = MAX_INSTANCES;
        if (gPipeline && total > 0) {
            memcpy(gInstanceBuf.contents, fd.instances, total * INSTANCE_STRIDE);
            uint32_t c1 = fd.count > total ? total : fd.count;
            if (c1 > 0) {
                [enc setRenderPipelineState:gPipeline];
                [enc setVertexBytes:&u length:sizeof(u) atIndex:1];
                [enc setFragmentTexture:gAtlas atIndex:0];
                CGSize dds = CGSizeMake(w, h);
                if (fd.pane_range_count > 0) {
                    for (uint32_t pi = 0; pi < fd.pane_range_count; pi++) {
                        const PaneRange *pr = &fd.pane_ranges[pi];
                        uint32_t end = pr->offset + pr->count;
                        if (end > c1) end = c1;
                        if (pr->offset >= end) continue;
                        NSUInteger sx = (NSUInteger)pr->x;
                        NSUInteger sy = (NSUInteger)pr->y;
                        NSUInteger sw = (NSUInteger)pr->w;
                        NSUInteger sh = (NSUInteger)pr->h;
                        if (sw == 0 || sh == 0) continue;
                        if (sx + sw > (NSUInteger)dds.width)
                            sw = (NSUInteger)dds.width > sx ? (NSUInteger)dds.width - sx : 0;
                        if (sy + sh > (NSUInteger)dds.height)
                            sh = (NSUInteger)dds.height > sy ? (NSUInteger)dds.height - sy : 0;
                        if (sw == 0 || sh == 0) continue;
                        MTLScissorRect scissor = {sx, sy, sw, sh};
                        [enc setScissorRect:scissor];
                        [enc setVertexBuffer:gInstanceBuf
                                      offset:pr->offset * INSTANCE_STRIDE
                                     atIndex:0];
                        [enc drawPrimitives:MTLPrimitiveTypeTriangleStrip
                                vertexStart:0
                                vertexCount:4
                              instanceCount:end - pr->offset];
                    }
                    MTLScissorRect full = {0, 0, (NSUInteger)dds.width, (NSUInteger)dds.height};
                    [enc setScissorRect:full];
                } else {
                    [enc setVertexBuffer:gInstanceBuf offset:0 atIndex:0];
                    [enc drawPrimitives:MTLPrimitiveTypeTriangleStrip
                            vertexStart:0 vertexCount:4 instanceCount:c1];
                }
            }
        }
        if (gSolidPipeline) {
            const float kDrawPx = 2.0f;
            for (uint32_t i = 0; i < fd.divider_count; i++) {
                const float *d = fd.dividers + i * 4;
                float dx = d[0], dy = d[1], dw = d[2], dh = d[3];
                if (dw <= dh) { dx += (dw - kDrawPx) * 0.5f; dw = kDrawPx; }
                else          { dy += (dh - kDrawPx) * 0.5f; dh = kDrawPx; }
                drawSolid(enc, CGSizeMake(w, h), dx, dy, dw, dh,
                          fd.sep_color[0], fd.sep_color[1], fd.sep_color[2], 1.0f);
            }
            for (uint32_t i = 0; i < fd.overlay_count; i++) {
                const float *o = fd.overlay + i * 7;
                drawSolid(enc, CGSizeMake(w, h), o[0], o[1], o[2], o[3], o[4], o[5], o[6], 1.0f);
            }
        }
        if (gPipeline && fd.palette_text_count > 0 && fd.count < total) {
            [enc setRenderPipelineState:gPipeline];
            [enc setVertexBuffer:gInstanceBuf offset:fd.count * INSTANCE_STRIDE atIndex:0];
            [enc setVertexBytes:&u length:sizeof(u) atIndex:1];
            [enc setFragmentTexture:gAtlas atIndex:0];
            [enc drawPrimitives:MTLPrimitiveTypeTriangleStrip
                    vertexStart:0 vertexCount:4 instanceCount:(total - fd.count)];
        }
        [enc endEncoding];

        id<MTLBlitCommandEncoder> blit = [cb blitCommandEncoder];
        [blit synchronizeResource:tex];
        [blit endEncoding];
        [cb commit];
        [cb waitUntilCompleted];

        size_t bpr = (size_t)w * 4;
        uint8_t *px = malloc(bpr * h);
        [tex getBytes:px bytesPerRow:bpr fromRegion:MTLRegionMake2D(0, 0, w, h) mipmapLevel:0];

        CGColorSpaceRef cs = CGColorSpaceCreateDeviceRGB();
        CGBitmapInfo bi = kCGBitmapByteOrder32Little | kCGImageAlphaNoneSkipFirst;
        CGContextRef ctx = CGBitmapContextCreate(px, w, h, 8, bpr, cs, bi);
        CGImageRef img = CGBitmapContextCreateImage(ctx);

        CFStringRef cfpath = CFStringCreateWithCString(NULL, path, kCFStringEncodingUTF8);
        CFURLRef url = CFURLCreateWithFileSystemPath(NULL, cfpath, kCFURLPOSIXPathStyle, false);
        CGImageDestinationRef dest =
            CGImageDestinationCreateWithURL(url, (__bridge CFStringRef)UTTypePNG.identifier, 1, NULL);
        CGImageDestinationAddImage(dest, img, NULL);
        CGImageDestinationFinalize(dest);

        CFRelease(dest);
        CFRelease(url);
        CFRelease(cfpath);
        CGImageRelease(img);
        CGContextRelease(ctx);
        CGColorSpaceRelease(cs);
        free(px);
    }
}

// Vertically center the traffic-light buttons inside our compact bar.
static void layoutTrafficLights(NSWindow *win) {
    NSButton *btns[3] = {
        [win standardWindowButton:NSWindowCloseButton],
        [win standardWindowButton:NSWindowMiniaturizeButton],
        [win standardWindowButton:NSWindowZoomButton],
    };
    if (!btns[0]) return;
    NSView *tbar = btns[0].superview;
    CGFloat tbarH = tbar.frame.size.height;
    CGFloat bh = btns[0].frame.size.height;
    CGFloat y = tbarH - BAR_H_PT / 2.0 - bh / 2.0;
    CGFloat x = 13.0, spacing = 20.0;
    for (int i = 0; i < 3; i++) {
        NSRect f = btns[i].frame;
        f.origin.x = x + i * spacing;
        f.origin.y = y;
        btns[i].frame = f;
    }
}

@interface AnvilView : NSView
@end

@implementation AnvilView
- (CALayer *)makeBackingLayer {
    return gLayer;
}
- (void)setFrameSize:(NSSize)size {
    [super setFrameSize:size];
    CGFloat scale = self.window.backingScaleFactor ?: 2.0;
    // Disable the implicit CA animation on drawableSize so the layer does not
    // stretch the old frame while the grid reflows — that is the resize glitch.
    [CATransaction begin];
    [CATransaction setDisableActions:YES];
    gLayer.drawableSize = CGSizeMake(size.width * scale, size.height * scale);
    [CATransaction commit];
    if (self.window) layoutTrafficLights(self.window);
    // Render synchronously during live resize: the timer is suspended while the
    // run loop is in event-tracking mode, so without this the content freezes
    // and tears until the drag ends.
    if (self.window && gLayer.device) render();
}
- (BOOL)acceptsFirstResponder {
    return YES;
}
- (void)sendMouse:(NSEvent *)e kind:(int)kind {
    NSPoint p = [self convertPoint:e.locationInWindow fromView:nil];
    CGFloat scale = self.window.backingScaleFactor;
    anvil_mouse(kind, (float)(p.x * scale),
                (float)((self.bounds.size.height - p.y) * scale));
}
- (void)mouseDown:(NSEvent *)e {
    if (e.modifierFlags & NSEventModifierFlagCommand) {
        NSPoint p = [self convertPoint:e.locationInWindow fromView:nil];
        CGFloat scale = self.window.backingScaleFactor;
        float px = (float)(p.x * scale);
        float py = (float)((self.bounds.size.height - p.y) * scale);
        const char *uri_ptr = NULL;
        size_t uri_len = 0;
        if (anvil_link_at(px, py, &uri_ptr, &uri_len) && uri_ptr && uri_len > 0) {
            NSString *uriStr = [[NSString alloc] initWithBytes:uri_ptr length:uri_len
                                                      encoding:NSUTF8StringEncoding];
            NSURL *url = uriStr ? [NSURL URLWithString:uriStr] : nil;
            NSString *scheme = url.scheme.lowercaseString;
            if (url && ([scheme isEqualToString:@"http"] ||
                        [scheme isEqualToString:@"https"] ||
                        [scheme isEqualToString:@"file"])) {
                [[NSWorkspace sharedWorkspace] openURL:url];
            }
            return;
        }
    }
    [self sendMouse:e kind:0];
}
- (void)mouseDragged:(NSEvent *)e {
    [self sendMouse:e kind:1];
}
- (void)mouseUp:(NSEvent *)e {
    [self sendMouse:e kind:2];
}
- (void)copySelection {
    size_t n = 0;
    const char *txt = anvil_copy(&n);
    if (n == 0) return;
    NSString *str = [[NSString alloc] initWithBytes:txt length:n encoding:NSUTF8StringEncoding];
    NSPasteboard *pb = [NSPasteboard generalPasteboard];
    [pb clearContents];
    [pb setString:str forType:NSPasteboardTypeString];
}
- (void)pasteClipboard {
    NSString *str = [[NSPasteboard generalPasteboard] stringForType:NSPasteboardTypeString];
    const char *u = str.UTF8String;
    if (u) anvil_paste(u, strlen(u));
    render();
}
- (void)keyDown:(NSEvent *)e {
    NSString *s = e.characters;
    NSEventModifierFlags f = e.modifierFlags;
    NSString *im = e.charactersIgnoringModifiers;
    unichar ich = im.length ? [im characterAtIndex:0] : 0;
    unichar ilc = (ich >= 'A' && ich <= 'Z') ? ich + 32 : ich;
    BOOL cmd = (f & NSEventModifierFlagCommand) != 0;

    // Cmd+N opens a new window (separate process, no session restore/save),
    // starting in the focused pane's cwd when known.
    if (cmd && ilc == 'n') {
        NSString *exe = [[NSBundle mainBundle] executablePath];
        if (!exe) exe = [[NSProcessInfo processInfo].arguments firstObject];
        if (exe) {
            NSMutableArray *args = [@[@"--new"] mutableCopy];
            char cbuf[1024];
            size_t cn = anvil_focused_cwd(cbuf, sizeof(cbuf));
            if (cn > 0) {
                NSString *cwd = [[NSString alloc] initWithBytes:cbuf length:cn encoding:NSUTF8StringEncoding];
                if (cwd) [args addObject:cwd];
            }
            NSTask *task = [[NSTask alloc] init];
            task.launchPath = exe;
            task.arguments = args;
            [task launch];
        }
        return;
    }
    // Cmd+K toggles the command palette from any state.
    if (cmd && ilc == 'k') { anvil_palette_toggle(); return; }
    // Cmd+F toggles scrollback search from any state.
    if (cmd && ilc == 'f') { anvil_search_toggle(); return; }
    // Cmd+/ toggles the keyboard shortcut cheatsheet from any state.
    if (cmd && ilc == '/') { anvil_help_toggle(); return; }
    // Cmd+Shift+Space toggles copy mode from any state.
    {
        BOOL shift = (f & NSEventModifierFlagShift) != 0;
        if (cmd && shift && ich == ' ') { anvil_copy_mode_toggle(); return; }
    }
    // Cmd+G toggles the Caldera run-detail drawer from any state.
    if (cmd && ilc == 'g') { anvil_caldera_drawer_toggle(); return; }

    // While the run-detail drawer is open it captures nav keys; PTY sees nothing.
    if (anvil_caldera_drawer_open()) {
        unichar ch = s.length ? [s characterAtIndex:0] : 0;
        if (ch == 0x1b) { anvil_caldera_drawer_key(0); return; }
        if (ch == NSUpArrowFunctionKey)   { anvil_caldera_drawer_key(1); return; }
        if (ch == NSDownArrowFunctionKey) { anvil_caldera_drawer_key(2); return; }
        return; // swallow everything else
    }

    // While the cheatsheet is open it captures all keys; the PTY sees nothing.
    if (anvil_help_open()) {
        unichar ch = s.length ? [s characterAtIndex:0] : 0;
        if (ch == 0x1b) { anvil_help_key(0); return; } // esc closes
        return; // swallow everything else
    }

    // While copy mode is open it captures all keys; the PTY sees nothing.
    if (anvil_copy_mode_open()) {
        BOOL shift = (f & NSEventModifierFlagShift) != 0;
        if (cmd) return; // swallow other Cmd shortcuts
        unichar ch = s.length ? [s characterAtIndex:0] : 0;
        switch (ch) {
            case 0x1b: anvil_copy_mode_key(0); return; // esc
            case '\r': case '\n': anvil_copy_mode_key(2); return; // enter = copy+exit
            case NSUpArrowFunctionKey:   anvil_copy_mode_key(3); return;
            case NSDownArrowFunctionKey: anvil_copy_mode_key(4); return;
            case NSLeftArrowFunctionKey:  anvil_copy_mode_key(5); return;
            case NSRightArrowFunctionKey: anvil_copy_mode_key(6); return;
        }
        unichar lch = (ch >= 'A' && ch <= 'Z') ? ch + 32 : ch;
        if (lch == 'q') { anvil_copy_mode_key(0); return; }
        if (lch == 'v') { anvil_copy_mode_key(1); return; }
        if (lch == 'y') { anvil_copy_mode_key(2); return; }
        if (lch == 'k') { anvil_copy_mode_key(3); return; }
        if (lch == 'j') { anvil_copy_mode_key(4); return; }
        if (lch == 'h') { anvil_copy_mode_key(5); return; }
        if (lch == 'l') { anvil_copy_mode_key(6); return; }
        if (lch == 'g' && !shift) { anvil_copy_mode_key(7); return; }
        if (lch == 'g' && shift)  { anvil_copy_mode_key(8); return; } // G
        // Ctrl+U = 0x15, Ctrl+D = 0x04
        if (ch == 0x15) { anvil_copy_mode_key(9);  return; }
        if (ch == 0x04) { anvil_copy_mode_key(10); return; }
        if (lch == 'w') { anvil_copy_mode_key(11); return; }
        if (lch == 'b') { anvil_copy_mode_key(12); return; }
        return; // swallow everything else
    }

    // While search is open it captures all keys; the PTY sees nothing.
    if (anvil_search_open()) {
        BOOL shift = (f & NSEventModifierFlagShift) != 0;
        if (cmd) return; // swallow other shortcuts while open
        unichar ch = s.length ? [s characterAtIndex:0] : 0;
        switch (ch) {
            case 0x1b: anvil_search_key(0); return; // esc
            case '\r': case '\n': anvil_search_key(shift ? 2 : 1); return; // enter = next, shift+enter = prev
            case NSUpArrowFunctionKey:   anvil_search_key(2); return; // prev
            case NSDownArrowFunctionKey: anvil_search_key(1); return; // next
            case 0x7f: case 0x08: anvil_search_key(4); return; // backspace
            case '\t': anvil_search_key(5); return; // tab = toggle regex mode
        }
        if (ch >= 0x20 && ch < 0x7f) { anvil_search_char((unsigned char)ch); return; }
        return;
    }

    // While the palette is open it captures all keys; the PTY sees nothing.
    if (anvil_palette_open()) {
        if (cmd) return; // swallow other shortcuts while open
        unichar ch = s.length ? [s characterAtIndex:0] : 0;
        switch (ch) {
            case 0x1b: anvil_palette_key(0); return; // esc
            case '\r': case '\n': anvil_palette_key(1); return; // enter
            case NSUpArrowFunctionKey:   anvil_palette_key(2); return;
            case NSDownArrowFunctionKey: anvil_palette_key(3); return;
            case 0x7f: case 0x08: anvil_palette_key(4); return; // backspace
        }
        if (ch >= 0x20 && ch < 0x7f) { anvil_palette_char((unsigned char)ch); return; }
        return;
    }

    if (anvil_cfg_error_open()) {
        unichar ch = s.length ? [s characterAtIndex:0] : 0;
        if (ch == 0x1b) { anvil_cfg_error_dismiss(); return; }
    }

    if (f & NSEventModifierFlagCommand) {
        unichar ch = ich;
        BOOL shift = (f & NSEventModifierFlagShift) != 0;
        if (f & NSEventModifierFlagOption) {
            switch (ch) {
                case NSLeftArrowFunctionKey:  anvil_focus_dir(0); return;
                case NSRightArrowFunctionKey: anvil_focus_dir(1); return;
                case NSUpArrowFunctionKey:    anvil_focus_dir(2); return;
                case NSDownArrowFunctionKey:  anvil_focus_dir(3); return;
            }
        }
        // Shift+arrow resizes the focused pane; plain Cmd+up/down jumps prompts.
        switch (ch) {
            case NSLeftArrowFunctionKey:  if (shift) { anvil_resize_pane(0); return; } break;
            case NSRightArrowFunctionKey: if (shift) { anvil_resize_pane(1); return; } break;
            case NSUpArrowFunctionKey:    if (shift) anvil_resize_pane(2); else anvil_jump_prompt(-1); return;
            case NSDownArrowFunctionKey:  if (shift) anvil_resize_pane(3); else anvil_jump_prompt(1); return;
        }
        if (ch >= '1' && ch <= '9') { anvil_select_tab((int)(ch - '1')); return; }
        if (ch == ']' || ch == '}') { anvil_cycle_tab(1); return; }
        if (ch == '[' || ch == '{') { anvil_cycle_tab(-1); return; }
        if (ch == '\r' || ch == '\n') { if (shift) { anvil_zoom_toggle(); return; } }
        if (ch == '=' || ch == '+') { anvil_balance_panes(); return; }
        unichar lc = (ch >= 'A' && ch <= 'Z') ? ch + 32 : ch;
        if (lc == 'c') [self copySelection];
        else if (lc == 'v') [self pasteClipboard];
        else if (lc == 'd') anvil_split(shift ? 1 : 0);
        else if (lc == 't') anvil_new_tab();
        else if (lc == 'w') { if (shift) anvil_close_tab(); else anvil_close_pane(); }
        else if (lc == 'r' && !shift) anvil_respawn();
        return;
    }
    if (s.length == 1) {
        unichar ch = [s characterAtIndex:0];
        const char *seq = NULL;
        switch (ch) {
            case NSUpArrowFunctionKey:    seq = "\x1b[A"; break;
            case NSDownArrowFunctionKey:  seq = "\x1b[B"; break;
            case NSRightArrowFunctionKey: seq = "\x1b[C"; break;
            case NSLeftArrowFunctionKey:  seq = "\x1b[D"; break;
        }
        if (seq) {
            anvil_input(seq, 3);
            render();
            return;
        }
    }
    const char *u = s.UTF8String;
    if (u) anvil_input(u, strlen(u));
    render();
}
- (void)scrollWheel:(NSEvent *)e {
    // Accumulate fractional scroll so small trackpad swipes are not truncated to
    // zero (jerky) and large ones do not lose their remainder between events.
    static CGFloat acc = 0.0;
    if (e.phase == NSEventPhaseBegan) acc = 0.0;
    CGFloat dy = e.scrollingDeltaY;
    if (dy == 0) return;
    // Precise (trackpad) deltas are in points; coarse mouse-wheel deltas already
    // approximate line steps.
    acc += e.hasPreciseScrollingDeltas ? dy / 8.0 : dy;
    int lines = (int)acc;
    if (lines == 0) return;
    acc -= (CGFloat)lines;
    anvil_scroll(lines);
    render();
}
@end

@interface AnvilTick : NSObject
@end

@implementation AnvilTick
- (void)tick:(id)sender {
    (void)sender;
    render();
}
@end

@interface AnvilController : NSObject <NSApplicationDelegate>
@end

@implementation AnvilController
- (void)setTheme:(NSMenuItem *)sender {
    anvil_set_theme_mode((int)sender.tag);
    for (NSMenuItem *item in sender.menu.itemArray)
        item.state = (item == sender) ? NSControlStateValueOn : NSControlStateValueOff;
    applyAppearance();
}
- (void)osAppearanceChanged:(NSNotification *)n {
    (void)n;
    anvil_set_os_dark(osIsDark() ? 1 : 0);
    applyAppearance();
}
- (void)applicationWillTerminate:(NSNotification *)n {
    (void)n;
    anvil_save_session();
}
- (void)applicationDidBecomeActive:(NSNotification *)n {
    (void)n;
    anvil_ipc_focus();
    anvil_force_render();
}
@end

static AnvilController *gController;

static void buildMenu(void) {
    NSMenu *bar = [[NSMenu alloc] init];
    [NSApp setMainMenu:bar];

    NSMenuItem *appItem = [[NSMenuItem alloc] init];
    [bar addItem:appItem];
    NSMenu *appMenu = [[NSMenu alloc] init];
    [appMenu addItemWithTitle:@"Quit Anvil"
                       action:@selector(terminate:)
                keyEquivalent:@"q"];
    appItem.submenu = appMenu;

    NSMenuItem *viewItem = [[NSMenuItem alloc] init];
    [bar addItem:viewItem];
    NSMenu *viewMenu = [[NSMenu alloc] initWithTitle:@"View"];
    NSMenu *themeMenu = [[NSMenu alloc] initWithTitle:@"Theme"];
    struct {
        NSString *title;
        int tag;
    } modes[3] = {{@"System", 0}, {@"Light", 1}, {@"Dark", 2}};
    for (int i = 0; i < 3; i++) {
        NSMenuItem *mi = [[NSMenuItem alloc] initWithTitle:modes[i].title
                                                    action:@selector(setTheme:)
                                             keyEquivalent:@""];
        mi.tag = modes[i].tag;
        mi.target = gController;
        mi.state = (modes[i].tag == 0) ? NSControlStateValueOn : NSControlStateValueOff;
        [themeMenu addItem:mi];
    }
    NSMenuItem *themeItem = [[NSMenuItem alloc] initWithTitle:@"Theme"
                                                       action:nil
                                                keyEquivalent:@""];
    themeItem.submenu = themeMenu;
    [viewMenu addItem:themeItem];
    viewItem.submenu = viewMenu;
}

// Post a macOS user notification when the app is not frontmost. Requires the
// bundled app (io.brzrkr.anvil); no-op when running unbundled so --dump never
// crashes. Authorization is requested once per launch; subsequent calls are
// fire-and-forget.
static BOOL gNotifyAuthorized = NO;
static BOOL gNotifyRequested = NO;

void anvil_notify(const char *title, const char *body) {
    if ([NSApp isActive]) return;

    // Only works in a properly bundled app; bail out if there is no bundle id.
    NSString *bundleId = [[NSBundle mainBundle] bundleIdentifier];
    if (!bundleId) return;

    UNUserNotificationCenter *center = [UNUserNotificationCenter currentNotificationCenter];

    if (!gNotifyRequested) {
        gNotifyRequested = YES;
        [center requestAuthorizationWithOptions:(UNAuthorizationOptionAlert | UNAuthorizationOptionSound)
                              completionHandler:^(BOOL granted, NSError *err) {
            (void)err;
            gNotifyAuthorized = granted;
        }];
        // Return now; the authorization callback fires asynchronously. The next
        // completed-command event (if any) will reach the authorized path.
        return;
    }

    if (!gNotifyAuthorized) return;

    UNMutableNotificationContent *content = [[UNMutableNotificationContent alloc] init];
    content.title = [NSString stringWithUTF8String:title];
    content.body  = [NSString stringWithUTF8String:body];

    NSString *ident = [NSString stringWithFormat:@"anvil.cmd.%f", [NSDate timeIntervalSinceReferenceDate]];
    UNNotificationRequest *req = [UNNotificationRequest requestWithIdentifier:ident
                                                                      content:content
                                                                      trigger:nil];
    [center addNotificationRequest:req withCompletionHandler:nil];
}

// Write UTF-8 text to the system pasteboard. Called by Zig to fulfill OSC 52
// clipboard-set requests from the running program.
void anvil_pasteboard_write(const char *p, size_t n) {
    NSString *str = [[NSString alloc] initWithBytes:p length:n encoding:NSUTF8StringEncoding];
    if (!str) return;
    NSPasteboard *pb = [NSPasteboard generalPasteboard];
    [pb clearContents];
    [pb setString:str forType:NSPasteboardTypeString];
}

void anvil_run(void) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];

        // Dock icon for the bare binary (the .app bundle uses AppIcon.icns).
        size_t ilen = 0;
        const uint8_t *idata = anvil_icon_data(&ilen);
        NSImage *icon = [[NSImage alloc] initWithData:[NSData dataWithBytes:idata length:ilen]];
        if (icon) NSApp.applicationIconImage = icon;

        gDevice = MTLCreateSystemDefaultDevice();
        gQueue = [gDevice newCommandQueue];

        gLayer = [CAMetalLayer layer];
        gLayer.device = gDevice;
        gLayer.pixelFormat = MTLPixelFormatBGRA8Unorm;
        gLayer.framebufferOnly = YES;

        buildPipeline();
        buildAtlas();

        // Rasterize common TUI glyphs before the first frame to avoid
        // per-glyph stall during initial paint (vim, lazygit, btop).
        {
            const void *pw_ptr = NULL;
            uint32_t pw_count = 0;
            anvil_prewarm_atlas(&pw_ptr, &pw_count);
            const uint32_t *pg = (const uint32_t *)pw_ptr;
            for (uint32_t i = 0; i < pw_count; i++) {
                rasterizeGlyph(pg[i * 3], pg[i * 3 + 1], pg[i * 3 + 2]);
            }
        }

        NSRect frame = NSMakeRect(0, 0, 800, 500);
        NSWindow *win = [[NSWindow alloc]
            initWithContentRect:frame
            styleMask:(NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                       NSWindowStyleMaskResizable | NSWindowStyleMaskMiniaturizable |
                       NSWindowStyleMaskFullSizeContentView)
            backing:NSBackingStoreBuffered
            defer:NO];
        [win setTitle:@"Anvil"];
        win.titleVisibility = NSWindowTitleHidden;
        win.titlebarAppearsTransparent = YES;
        gWindow = win;

        gController = [[AnvilController alloc] init];
        NSApp.delegate = gController;
        anvil_set_os_dark(osIsDark() ? 1 : 0);
        buildMenu();
        applyAppearance();
        [[NSDistributedNotificationCenter defaultCenter]
            addObserver:gController
               selector:@selector(osAppearanceChanged:)
                   name:@"AppleInterfaceThemeChangedNotification"
                 object:nil];

        // Translucency setup: window and layer stay non-opaque so that when
        // bg_alpha < 1.0 the vibrancy blur shows through. At alpha 1.0 the
        // opaque-colored clear fully covers the blur, so the default look is
        // unchanged and no restart is needed when background_opacity changes.
        win.opaque = NO;
        win.backgroundColor = [NSColor clearColor];
        gLayer.opaque = NO;

        NSView *contentHost = [[NSView alloc] initWithFrame:frame];
        contentHost.autoresizesSubviews = YES;
        [win setContentView:contentHost];

        NSVisualEffectView *vev = [[NSVisualEffectView alloc] initWithFrame:frame];
        vev.material = NSVisualEffectMaterialUnderWindowBackground;
        vev.blendingMode = NSVisualEffectBlendingModeBehindWindow;
        vev.state = NSVisualEffectStateActive;
        vev.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        [contentHost addSubview:vev];

        AnvilView *view = [[AnvilView alloc] initWithFrame:frame];
        view.wantsLayer = YES;
        view.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        gLayer.frame = view.bounds;
        gLayer.drawableSize = CGSizeMake(frame.size.width * 2, frame.size.height * 2);
        [contentHost addSubview:view];
        // Restore the last window position/size; center only on first launch.
        win.frameAutosaveName = @"AnvilMainWindow";
        if (![win setFrameUsingName:@"AnvilMainWindow"]) [win center];
        [win makeKeyAndOrderFront:nil];
        [win makeFirstResponder:view];
        [NSApp activateIgnoringOtherApps:YES];
        layoutTrafficLights(win);

        AnvilTick *tick = [[AnvilTick alloc] init];
        // macOS 14+: use CADisplayLink via NSView for ProMotion-aware vsync.
        // macOS 13: fall back to 60Hz NSTimer (CommonModes keeps it firing
        // during live resize and window drags).
        if (@available(macOS 14.0, *)) {
            CADisplayLink *dl = [view displayLinkWithTarget:tick selector:@selector(tick:)];
            [dl addToRunLoop:[NSRunLoop currentRunLoop] forMode:NSRunLoopCommonModes];
        } else {
            NSTimer *timer = [NSTimer timerWithTimeInterval:1.0 / 60.0
                                                     target:tick
                                                   selector:@selector(tick:)
                                                   userInfo:nil
                                                    repeats:YES];
            [[NSRunLoop currentRunLoop] addTimer:timer forMode:NSRunLoopCommonModes];
        }

        [NSApp run];
    }
}
