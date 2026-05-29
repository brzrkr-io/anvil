#import <Cocoa/Cocoa.h>
#import <QuartzCore/CAMetalLayer.h>
#import <Metal/Metal.h>

static id<MTLDevice> gDevice;
static id<MTLCommandQueue> gQueue;
static CAMetalLayer *gLayer;

static void render(void) {
    id<CAMetalDrawable> drawable = [gLayer nextDrawable];
    if (!drawable) return;

    MTLRenderPassDescriptor *rp = [MTLRenderPassDescriptor renderPassDescriptor];
    rp.colorAttachments[0].texture = drawable.texture;
    rp.colorAttachments[0].loadAction = MTLLoadActionClear;
    rp.colorAttachments[0].storeAction = MTLStoreActionStore;
    rp.colorAttachments[0].clearColor = MTLClearColorMake(0.05, 0.06, 0.08, 1.0);

    id<MTLCommandBuffer> cb = [gQueue commandBuffer];
    id<MTLRenderCommandEncoder> enc = [cb renderCommandEncoderWithDescriptor:rp];
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

        NSRect frame = NSMakeRect(0, 0, 800, 500);
        NSWindow *win = [[NSWindow alloc]
            initWithContentRect:frame
            styleMask:(NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                       NSWindowStyleMaskResizable | NSWindowStyleMaskMiniaturizable)
            backing:NSBackingStoreBuffered
            defer:NO];
        [win setTitle:@"Anvil"];

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
