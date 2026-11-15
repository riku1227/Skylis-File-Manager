// Native Quick Look ownership follows Apple's documented responder-chain contract.
// Compile with ARC; the shared panel's dataSource is unretained, so activeController
// keeps the provider alive until both the panel and responder chain release it.
#import <AppKit/AppKit.h>
#import <Quartz/Quartz.h>
#include <stdint.h>
#include <string.h>

@interface SkylisQuickLookController : NSResponder <QLPreviewPanelDataSource>
@property(nonatomic, strong) NSURL *item;
@property(nonatomic, weak) NSWindow *owner;
@property(nonatomic, weak) NSResponder *previousResponder;
@property(nonatomic) void *viewAddress;
- (void)ownerWillClose:(NSNotification *)notification;
@end

static SkylisQuickLookController *activeController;

static void closeActive(void) {
    SkylisQuickLookController *controller = activeController;
    if (!controller) return;
    activeController = nil;
    [[NSNotificationCenter defaultCenter] removeObserver:controller];
    if ([QLPreviewPanel sharedPreviewPanelExists]) {
        QLPreviewPanel *panel = [QLPreviewPanel sharedPreviewPanel];
        // A different window or framework may have taken over the shared panel.
        // Never mutate or hide that controller's preview during our cleanup.
        if (panel.currentController == controller) {
            if (panel.dataSource == controller) panel.dataSource = nil;
            [panel orderOut:nil];
        }
    }
    NSWindow *owner = controller.owner;
    if (owner.nextResponder == controller) owner.nextResponder = controller.previousResponder;
    controller.nextResponder = nil;
    controller.item = nil;
}

@implementation SkylisQuickLookController
- (BOOL)acceptsPreviewPanelControl:(QLPreviewPanel *)panel {
    (void)panel;
    return self == activeController && self.item != nil;
}
- (void)beginPreviewPanelControl:(QLPreviewPanel *)panel {
    panel.dataSource = self;
}
- (void)endPreviewPanelControl:(QLPreviewPanel *)panel {
    if (panel.dataSource == self) panel.dataSource = nil;
}
- (NSInteger)numberOfPreviewItemsInPreviewPanel:(QLPreviewPanel *)panel {
    (void)panel;
    return self.item ? 1 : 0;
}
- (id<QLPreviewItem>)previewPanel:(QLPreviewPanel *)panel previewItemAtIndex:(NSInteger)index {
    (void)panel;
    return index == 0 ? self.item : nil;
}
- (void)ownerWillClose:(NSNotification *)notification {
    if (notification.object == self.owner && self == activeController) closeActive();
}
@end

int32_t skylis_quicklook_show(void *viewAddress, const char *path) {
    if (![NSThread isMainThread]) return -1;
    if (!viewAddress || !path || path[0] != '/') return -2;
    @try {
        NSView *view = (__bridge NSView *)viewAddress;
        NSWindow *owner = view.window;
        if (!owner) return -2;
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        if (!url) return -2;
        if (!activeController || activeController.owner != owner) {
            closeActive();
            SkylisQuickLookController *controller = [[SkylisQuickLookController alloc] init];
            controller.owner = owner;
            controller.viewAddress = viewAddress;
            controller.previousResponder = owner.nextResponder;
            controller.nextResponder = owner.nextResponder;
            owner.nextResponder = controller;
            activeController = controller;
            [[NSNotificationCenter defaultCenter] addObserver:controller
                selector:@selector(ownerWillClose:) name:NSWindowWillCloseNotification object:owner];
        }
        activeController.item = url;
        QLPreviewPanel *panel = [QLPreviewPanel sharedPreviewPanel];
        [panel updateController];
        if (panel.currentController != activeController || panel.dataSource != activeController) {
            // Do not assign outside beginPreviewPanelControl: that would bypass
            // the system's responder-chain ownership and break other controllers.
            closeActive();
            return -3;
        }
        [panel reloadData];
        panel.currentPreviewItemIndex = 0;
        [panel makeKeyAndOrderFront:nil];
        return 0;
    } @catch (NSException *exception) {
        (void)exception;
        closeActive();
        return -3;
    }
}

// A render/selection refresh cannot open or raise a closed native panel. Only
// skylis_quicklook_show, called from an explicit user action, may do that.
int32_t skylis_quicklook_update(void *viewAddress, const char *path) {
    if (![NSThread isMainThread]) return -1;
    if (!viewAddress || !path || path[0] != '/') return -2;
    @try {
        if (!activeController || activeController.viewAddress != viewAddress ||
            ![QLPreviewPanel sharedPreviewPanelExists]) return 0;
        QLPreviewPanel *panel = [QLPreviewPanel sharedPreviewPanel];
        if (!panel.isVisible || panel.currentController != activeController) {
            closeActive();
            return 0;
        }
        const char *previousPath = activeController.item.fileSystemRepresentation;
        if (previousPath && strcmp(previousPath, path) == 0) return 1;
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path isDirectory:NO relativeToURL:nil];
        if (!url) return -2;
        activeController.item = url;
        [panel reloadData];
        panel.currentPreviewItemIndex = 0;
        return 1;
    } @catch (NSException *exception) { (void)exception; return -3; }
}

int32_t skylis_quicklook_close(void *viewAddress) {
    if (![NSThread isMainThread]) return -1;
    @try {
        // Address is only an identity comparison here; no stale NSView dereference.
        if (activeController.viewAddress == viewAddress) closeActive();
        return 0;
    } @catch (NSException *exception) { (void)exception; return -3; }
}

int32_t skylis_quicklook_visible(void *viewAddress) {
    if (![NSThread isMainThread]) return 0;
    @try {
        if (activeController.viewAddress != viewAddress || ![QLPreviewPanel sharedPreviewPanelExists]) return 0;
        QLPreviewPanel *panel = [QLPreviewPanel sharedPreviewPanel];
        return panel.isVisible && panel.currentController == activeController ? 1 : 0;
    } @catch (NSException *exception) { (void)exception; return 0; }
}

// Shared conversion for file pasteboard, Share and Services. URL creation does
// not query file metadata, hydrate placeholders, or invoke an external shell.
static NSArray<NSURL *> *fileURLs(const char *const *paths, size_t count) {
    if (!paths || count == 0 || count > 4096) return nil;
    NSMutableArray<NSURL *> *urls = [NSMutableArray arrayWithCapacity:count];
    for (size_t index = 0; index < count; ++index) {
        if (!paths[index] || paths[index][0] != '/') return nil;
        NSURL *url = [NSURL fileURLWithFileSystemRepresentation:paths[index] isDirectory:NO relativeToURL:nil];
        if (!url) return nil;
        [urls addObject:url];
    }
    return urls;
}

int32_t skylis_copy_files(const char *const *paths, size_t count) {
    if (![NSThread isMainThread]) return -1;
    @try {
        NSArray<NSURL *> *urls = fileURLs(paths, count);
        if (!urls) return -2;
        NSPasteboard *pasteboard = [NSPasteboard generalPasteboard];
        [pasteboard clearContents];
        return [pasteboard writeObjects:urls] ? 0 : -3;
    } @catch (NSException *exception) { (void)exception; return -3; }
}

static NSPoint viewPoint(NSView *view, double x, double y) {
    NSRect bounds = view.bounds;
    return NSMakePoint(NSMinX(bounds) + x, view.isFlipped ? NSMinY(bounds) + y : NSMaxY(bounds) - y);
}

@interface SkylisShareController : NSObject <NSSharingServicePickerDelegate>
@property(nonatomic, strong) NSSharingServicePicker *picker;
@property(nonatomic, weak) NSWindow *owner;
- (void)ownerWillClose:(NSNotification *)notification;
@end

static SkylisShareController *activeShare;
static void closeShare(void) {
    SkylisShareController *previous = activeShare;
    if (!previous) return;
    activeShare = nil;
    [[NSNotificationCenter defaultCenter] removeObserver:previous];
    previous.picker.delegate = nil;
    if ([previous.picker respondsToSelector:@selector(close)]) [previous.picker close];
}

@implementation SkylisShareController
- (void)sharingServicePicker:(NSSharingServicePicker *)picker didChooseSharingService:(NSSharingService *)service {
    (void)service;
    // The picker automatically executes only the service chosen by the user.
    if (activeShare == self && picker == self.picker) {
        [[NSNotificationCenter defaultCenter] removeObserver:self];
        self.picker.delegate = nil;
        activeShare = nil;
    }
}
- (void)ownerWillClose:(NSNotification *)notification {
    if (notification.object == self.owner && activeShare == self) closeShare();
}
@end

int32_t skylis_show_share(void *viewAddress, const char *const *paths, size_t count, double x, double y) {
    if (![NSThread isMainThread]) return -1;
    @try {
        NSView *view = (__bridge NSView *)viewAddress;
        NSArray<NSURL *> *urls = fileURLs(paths, count);
        if (!view.window || !urls) return -2;
        closeShare();
        SkylisShareController *controller = [[SkylisShareController alloc] init];
        controller.owner = view.window;
        controller.picker = [[NSSharingServicePicker alloc] initWithItems:urls];
        controller.picker.delegate = controller;
        activeShare = controller;
        [[NSNotificationCenter defaultCenter] addObserver:controller selector:@selector(ownerWillClose:)
            name:NSWindowWillCloseNotification object:view.window];
        NSPoint point = viewPoint(view, x, y);
        [controller.picker showRelativeToRect:NSMakeRect(point.x, point.y, 1, 1) ofView:view preferredEdge:NSMinYEdge];
        return 0;
    } @catch (NSException *exception) { (void)exception; closeShare(); return -3; }
}

@interface SkylisFileServices : NSResponder <NSServicesMenuRequestor>
@property(nonatomic, strong) NSArray<NSURL *> *urls;
@end

@implementation SkylisFileServices
- (BOOL)acceptsFirstResponder { return YES; }
- (id)validRequestorForSendType:(NSPasteboardType)sendType returnType:(NSPasteboardType)returnType {
    BOOL fileType = [sendType isEqualToString:NSPasteboardTypeFileURL] || [sendType isEqualToString:NSFilenamesPboardType];
    if (self.urls.count && fileType && returnType == nil) return self;
    return [super validRequestorForSendType:sendType returnType:returnType];
}
- (BOOL)writeSelectionToPasteboard:(NSPasteboard *)pasteboard types:(NSArray<NSPasteboardType> *)types {
    if ([types containsObject:NSPasteboardTypeFileURL]) {
        [pasteboard clearContents];
        return [pasteboard writeObjects:self.urls];
    }
    if ([types containsObject:NSFilenamesPboardType]) {
        NSMutableArray<NSString *> *paths = [NSMutableArray arrayWithCapacity:self.urls.count];
        for (NSURL *url in self.urls) [paths addObject:url.path];
        [pasteboard declareTypes:@[NSFilenamesPboardType] owner:nil];
        return [pasteboard setPropertyList:paths forType:NSFilenamesPboardType];
    }
    return NO;
}
@end

int32_t skylis_show_services(void *viewAddress, const char *const *paths, size_t count, double x, double y) {
    if (![NSThread isMainThread]) return -1;
    static BOOL tracking = NO;
    if (tracking) return -3;
    NSView *view = (__bridge NSView *)viewAddress;
    NSWindow *owner = view.window;
    if (!owner) return -2;
    NSMenu *previousMenu = NSApp.servicesMenu;
    NSResponder *previousFirst = owner.firstResponder;
    SkylisFileServices *requestor = [[SkylisFileServices alloc] init];
    int32_t result = -3;
    tracking = YES;
    @try {
        requestor.urls = fileURLs(paths, count);
        if (!requestor.urls) return -2;
        // First-responder eligibility exists only while the native Services menu
        // tracks. We advertise no return types, so services cannot replace files
        // by returning unhandled pasteboard content to this application.
        requestor.nextResponder = previousFirst;
        if (![owner makeFirstResponder:requestor]) return -3;
        NSMenu *menu = [[NSMenu alloc] initWithTitle:@"サービス"];
        NSApp.servicesMenu = menu;
        [NSApp registerServicesMenuSendTypes:@[NSPasteboardTypeFileURL, NSFilenamesPboardType] returnTypes:@[]];
        [menu update];
        if (menu.numberOfItems == 0) {
            result = -4;
        } else {
            [menu popUpMenuPositioningItem:nil atLocation:viewPoint(view, x, y) inView:view];
            result = 0;
        }
    } @catch (NSException *exception) {
        (void)exception;
        result = -3;
    } @finally {
        NSApp.servicesMenu = previousMenu;
        if (owner.firstResponder == requestor) [owner makeFirstResponder:previousFirst];
        requestor.nextResponder = nil;
        tracking = NO;
    }
    return result;
}
