#import <Cocoa/Cocoa.h>
#import <QuartzCore/CAMetalLayer.h>
#import <Metal/Metal.h>

typedef struct {
    const void *instances;
    uint32_t count;
    float cell_w, cell_h, pad_x, pad_y;
} FrameData;

typedef struct {
    float cell[2];
    float pad[2];
    float viewport[2];
} Uniforms;

extern const char *anvil_shader_src(size_t *len);
extern void anvil_resize(float w, float h);
extern void anvil_frame(FrameData *out);

#define INSTANCE_STRIDE (11 * sizeof(float))
#define MAX_INSTANCES 60000

static id<MTLDevice> gDevice;
static id<MTLCommandQueue> gQueue;
static id<MTLRenderPipelineState> gPipeline;
static id<MTLBuffer> gInstanceBuf;
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
        };

        [enc setRenderPipelineState:gPipeline];
        [enc setVertexBuffer:gInstanceBuf offset:0 atIndex:0];
        [enc setVertexBytes:&u length:sizeof(u) atIndex:1];
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
