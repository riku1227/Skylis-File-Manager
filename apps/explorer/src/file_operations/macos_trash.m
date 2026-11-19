#import <Foundation/Foundation.h>
#include <stdbool.h>
#include <stdio.h>

static void write_error(char *message, size_t capacity, NSString *text) {
    if (message == NULL || capacity == 0) {
        return;
    }
    message[0] = '\0';
    const char *utf8 = text.UTF8String;
    if (utf8 != NULL) {
        snprintf(message, capacity, "%s", utf8);
    }
}

bool skylis_trash_item(const char *path, char *message, size_t capacity) {
    if (message != NULL && capacity > 0) {
        message[0] = '\0';
    }
    if (path == NULL) {
        return false;
    }
    @autoreleasepool {
        @try {
            NSURL *url = [NSURL fileURLWithFileSystemRepresentation:path
                                                        isDirectory:NO
                                                      relativeToURL:nil];
            if (url == nil) {
                return false;
            }
            NSError *error = nil;
            if ([[NSFileManager defaultManager] trashItemAtURL:url
                                            resultingItemURL:nil
                                                       error:&error]) {
                return true;
            }
            write_error(message, capacity, error.localizedDescription);
            return false;
        } @catch (NSException *exception) {
            write_error(message, capacity, exception.reason ?: exception.name);
            return false;
        }
    }
}
