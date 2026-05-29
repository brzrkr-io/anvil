#import <Cocoa/Cocoa.h>
#import <QuartzCore/CAMetalLayer.h>
#import <Metal/Metal.h>
#import <CoreText/CoreText.h>
#import <CoreGraphics/CoreGraphics.h>

typedef struct {
    const void *instances;
    uint32_t count;
    float cell_w, cell_h, pad_x, pad_y;
    float cell_uv[2];
} FrameData;

typedef struct {
    float cell[2];
    float pad[2];
    float viewport[2];
    float cell_uv[2];
} Uniforms;

typedef struct {
    uint32_t first, count, cols, rows;
    float pt_size;
} AtlasParams;

extern const char *anvil_shader_src(size_t *len);
extern void anvil_resize(float w, float h);
extern void anvil_frame(FrameData *out);
extern void anvil_atlas_params(AtlasParams *out);
extern void anvil_set_metrics(float cell_w, float cell_h);

#define INSTANCE_STRIDE (12 * sizeof(float))
#define MAX_INSTANCES 60000
#define ATLAS_SCALE 2.0

static id<MTLDevice> gDevice;
static id<MTLCommandQueue> gQueue;
static id<MTLRenderPipelineState> gPipeline;
static id<MTLBuffer> gInstanceBuf;
static id<MTLTexture> gAtlas;
static CAMetalLayer *gLayer;
static double gLastW, gLastH;

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

    gInstanceBuf = [gDevice newBufferWithLength:MAX_INSTANCES * INSTANCE_STRIDE
                                        options:MTLResourceStorageModeShared];
}

// Rasterize glyphs first..first+count into a cols x rows grid texture (R8).
// Atlas layout (grid, range) is decided by Zig; here is the CoreText ceremony.
static void buildAtlas(void) {
    AtlasParams ap = {0};
    anvil_atlas_params(&ap);

    CGFloat sz = ap.pt_size * ATLAS_SCALE;
    CTFontRef font = CTFontCreateWithName(CFSTR("Menlo"), sz, NULL);
    CGFloat ascent = CTFontGetAscent(font);
    CGFloat descent = CTFontGetDescent(font);
    CGFloat leading = CTFontGetLeading(font);

    UniChar mch = 'M';
    CGGlyph mg;
    CTFontGetGlyphsForCharacters(font, &mch, &mg, 1);
    CGSize adv;
    CTFontGetAdvancesForGlyphs(font, kCTFontOrientationHorizontal, &mg, &adv, 1);

    int gw = (int)ceil(adv.width);
    int gh = (int)ceil(ascent + descent + leading);
    int aw = gw * ap.cols;
    int ah = gh * ap.rows;
    anvil_set_metrics((float)gw, (float)gh);

    uint8_t *buf = calloc((size_t)aw * ah, 1);
    CGColorSpaceRef gray = CGColorSpaceCreateDeviceGray();
    CGContextRef ctx = CGBitmapContextCreate(buf, aw, ah, 8, aw, gray, kCGImageAlphaNone);
    CGContextSetGrayFillColor(ctx, 1.0, 1.0);

    for (uint32_t i = 0; i < ap.count; i++) {
        UniChar ch = (UniChar)(ap.first + i);
        CGGlyph g;
        if (!CTFontGetGlyphsForCharacters(font, &ch, &g, 1)) continue;
        int col = i % ap.cols;
        int row = i / ap.cols;
        CGPoint pt = CGPointMake(col * gw, ah - (row + 1) * gh + descent);
        CTFontDrawGlyphs(font, &g, &pt, 1, ctx);
    }

    MTLTextureDescriptor *td =
        [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatR8Unorm
                                                           width:aw
                                                          height:ah
                                                       mipmapped:NO];
    gAtlas = [gDevice newTextureWithDescriptor:td];
    [gAtlas replaceRegion:MTLRegionMake2D(0, 0, aw, ah)
              mipmapLevel:0
                withBytes:buf
              bytesPerRow:aw];

    CGContextRelease(ctx);
    CGColorSpaceRelease(gray);
    free(buf);
    CFRelease(font);
}

static void render(void) {
    CGSize ds = gLayer.drawableSize;
    if (ds.width <= 0 || ds.height <= 0) return;

    if (ds.width != gLastW || ds.height != gLastH) {
        gLastW = ds.width;
        gLastH = ds.height;
        anvil_resize((float)ds.width, (float)ds.height);
    }

    FrameData fd = {0};
    anvil_frame(&fd);

    id<CAMetalDrawable> drawable = [gLayer nextDrawable];
    if (!drawable) return;

    MTLRenderPassDescriptor *rp = [MTLRenderPassDescriptor renderPassDescriptor];
    rp.colorAttachments[0].texture = drawable.texture;
    rp.colorAttachments[0].loadAction = MTLLoadActionClear;
    rp.colorAttachments[0].storeAction = MTLStoreActionStore;
    rp.colorAttachments[0].clearColor = MTLClearColorMake(0.05, 0.06, 0.08, 1.0);

    id<MTLCommandBuffer> cb = [gQueue commandBuffer];
    id<MTLRenderCommandEncoder> enc = [cb renderCommandEncoderWithDescriptor:rp];

    if (gPipeline && fd.count > 0) {
        uint32_t count = fd.count > MAX_INSTANCES ? MAX_INSTANCES : fd.count;
        memcpy(gInstanceBuf.contents, fd.instances, count * INSTANCE_STRIDE);

        Uniforms u = {
            .cell = {fd.cell_w, fd.cell_h},
            .pad = {fd.pad_x, fd.pad_y},
            .viewport = {(float)ds.width, (float)ds.height},
            .cell_uv = {fd.cell_uv[0], fd.cell_uv[1]},
        };

        [enc setRenderPipelineState:gPipeline];
        [enc setVertexBuffer:gInstanceBuf offset:0 atIndex:0];
        [enc setVertexBytes:&u length:sizeof(u) atIndex:1];
        [enc setFragmentTexture:gAtlas atIndex:0];
        [enc drawPrimitives:MTLPrimitiveTypeTriangleStrip
                vertexStart:0
                vertexCount:4
              instanceCount:count];
    }

    [enc endEncoding];
    [cb presentDrawable:drawable];
    [cb commit];
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
    gLayer.drawableSize = CGSizeMake(size.width * scale, size.height * scale);
}
@end

@interface AnvilTick : NSObject
@end

@implementation AnvilTick
- (void)tick:(NSTimer *)t {
    (void)t;
    render();
}
@end

void anvil_run(void) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];

        gDevice = MTLCreateSystemDefaultDevice();
        gQueue = [gDevice newCommandQueue];

        gLayer = [CAMetalLayer layer];
        gLayer.device = gDevice;
        gLayer.pixelFormat = MTLPixelFormatBGRA8Unorm;
        gLayer.framebufferOnly = YES;

        buildPipeline();
        buildAtlas();

        NSRect frame = NSMakeRect(0, 0, 800, 500);
        NSWindow *win = [[NSWindow alloc]
            initWithContentRect:frame
            styleMask:(NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                       NSWindowStyleMaskResizable | NSWindowStyleMaskMiniaturizable |
                       NSWindowStyleMaskFullSizeContentView)
            backing:NSBackingStoreBuffered
            defer:NO];
        [win setTitle:@"Anvil"];
        win.titlebarAppearsTransparent = YES;
        win.titleVisibility = NSWindowTitleHidden;

        AnvilView *view = [[AnvilView alloc] initWithFrame:frame];
        view.wantsLayer = YES;
        gLayer.frame = view.bounds;
        gLayer.drawableSize = CGSizeMake(frame.size.width * 2, frame.size.height * 2);
        [win setContentView:view];
        [win center];
        [win makeKeyAndOrderFront:nil];
        [NSApp activateIgnoringOtherApps:YES];

        AnvilTick *tick = [[AnvilTick alloc] init];
        [NSTimer scheduledTimerWithTimeInterval:1.0 / 60.0
                                         target:tick
                                       selector:@selector(tick:)
                                       userInfo:nil
                                        repeats:YES];

        [NSApp run];
    }
}
