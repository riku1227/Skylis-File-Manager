// Only called on the bounded icon worker. No NSImage crosses the FFI boundary.
#import <AppKit/AppKit.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>
#include <string.h>
#include <stdint.h>

int32_t skylis_macos_icon(const char *path, int32_t directory, uint32_t size,
                         int32_t typeOnly, uint8_t *pixels) {
    if (!path || !pixels || size < 16 || size > 256) return 0;
    @autoreleasepool {
        @try {
            NSString *name = [[NSFileManager defaultManager]
                stringWithFileSystemRepresentation:path length:strlen(path)];
            if (!name) return 0;
            NSWorkspace *workspace = [NSWorkspace sharedWorkspace];
            // Type-only requests never query a real path, even for directories or
            // extensionless names. Copy workspace images before drawing/mutation.
            UTType *type = directory ? UTTypeFolder
                : ([UTType typeWithFilenameExtension:name.pathExtension] ?: UTTypeData);
            NSImage *image = typeOnly ? [workspace iconForContentType:type]
                                     : [workspace iconForFile:name];
            image = [image copy];
            if (!image) return 0;
            CGColorSpaceRef colorSpace = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
            CGContextRef bitmap = CGBitmapContextCreate(pixels, size, size, 8, size * 4,
                colorSpace, kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little);
            CGColorSpaceRelease(colorSpace);
            if (!bitmap) return 0;
            // Top-down BGRA8, matching GPUI RenderImage. Independent per-work
            // graphics context; never lockFocus on shared workspace images/views.
            CGContextTranslateCTM(bitmap, 0, size);
            CGContextScaleCTM(bitmap, 1, -1);
            [NSGraphicsContext saveGraphicsState];
            @try {
                [NSGraphicsContext setCurrentContext:
                    [NSGraphicsContext graphicsContextWithCGContext:bitmap flipped:YES]];
                [image drawInRect:NSMakeRect(0, 0, size, size) fromRect:NSZeroRect
                    operation:NSCompositingOperationCopy fraction:1 respectFlipped:YES hints:nil];
                CGContextFlush(bitmap);
            } @finally {
                [NSGraphicsContext restoreGraphicsState];
                CGContextRelease(bitmap);
            }
            // GPUI takes straight alpha despite the image::RgbaImage container.
            for (size_t i = 0; i < (size_t)size * size * 4; i += 4) {
                unsigned alpha = pixels[i + 3];
                for (size_t c = 0; c < 3; ++c)
                    pixels[i + c] = alpha ? MIN(255u, (pixels[i + c] * 255u + alpha / 2) / alpha) : 0;
            }
            return 1;
        } @catch (NSException *exception) { (void)exception; return 0; }
    }
}
