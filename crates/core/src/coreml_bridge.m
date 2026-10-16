#import <CoreML/CoreML.h>
#import <Foundation/Foundation.h>

@interface CraigCoreMlHandle : NSObject
@property(nonatomic, strong) MLModel *model;
@property(nonatomic, strong) NSURL *compiled;
@end

@implementation CraigCoreMlHandle
- (void)dealloc {
    // Core ML may prepare shape-specific execution lazily; retain its assets for the model lifetime.
    _model = nil;
    if (_compiled) [[NSFileManager defaultManager] removeItemAtURL:_compiled error:nil];
}
@end

// The Rust owner serializes predictions. No NSError or exception text crosses this boundary.
void *craig_coreml_load(const char *path, int units) {
    @autoreleasepool {
        @try {
            NSURL *url = [NSURL fileURLWithPath:[NSString stringWithUTF8String:path]];
            NSError *error = nil;
            CraigCoreMlHandle *handle = [CraigCoreMlHandle new];
            handle.compiled = [MLModel compileModelAtURL:url error:&error];
            if (!handle.compiled) return NULL;
            MLModelConfiguration *config = [MLModelConfiguration new];
            switch (units) {
                case 0: config.computeUnits = MLComputeUnitsAll; break;
                case 1: config.computeUnits = MLComputeUnitsCPUOnly; break;
                case 2: config.computeUnits = MLComputeUnitsCPUAndGPU; break;
                case 3: config.computeUnits = MLComputeUnitsCPUAndNeuralEngine; break;
                default: return NULL;
            }
            handle.model = [MLModel modelWithContentsOfURL:handle.compiled configuration:config error:&error];
            return handle.model ? (__bridge_retained void *)handle : NULL;
        } @catch (NSException *exception) { return NULL; }
    }
}

void craig_coreml_free(void *handle) {
    @autoreleasepool { (void)CFBridgingRelease(handle); }
}

int craig_coreml_predict(void *handle, const int32_t *features, size_t batch, size_t width, float *logits) {
    @autoreleasepool {
        @try {
            MLModel *model = ((__bridge CraigCoreMlHandle *)handle).model;
            NSError *error = nil;
            MLMultiArray *input = [[MLMultiArray alloc] initWithShape:@[@3, @(batch), @(width)]
                                                           dataType:MLMultiArrayDataTypeInt32 error:&error];
            if (!input) return 0;
            // Newly allocated MLMultiArray storage is contiguous; strides still govern indexing.
            int32_t *data = input.dataPointer;
            for (size_t c = 0; c < 3; c++)
                for (size_t b = 0; b < batch; b++)
                    for (size_t w = 0; w < width; w++)
                        data[c * input.strides[0].unsignedLongValue + b * input.strides[1].unsignedLongValue + w * input.strides[2].unsignedLongValue] = features[(c * batch + b) * width + w];
            MLDictionaryFeatureProvider *provider = [[MLDictionaryFeatureProvider alloc]
                initWithDictionary:@{@"features": [MLFeatureValue featureValueWithMultiArray:input]} error:&error];
            if (!provider) return 0;
            id<MLFeatureProvider> prediction = [model predictionFromFeatures:provider error:&error];
            MLMultiArray *output = [prediction featureValueForName:@"logits"].multiArrayValue;
            if (!output || output.dataType != MLMultiArrayDataTypeFloat32 || output.shape.count != 3 ||
                output.shape[0].unsignedLongValue != batch || output.shape[1].unsignedLongValue != width ||
                output.shape[2].unsignedLongValue != 2) return 0;
            const float *values = output.dataPointer;
            for (size_t b = 0; b < batch; b++)
                for (size_t w = 0; w < width; w++)
                    for (size_t c = 0; c < 2; c++)
                        logits[(b * width + w) * 2 + c] = values[b * output.strides[0].unsignedLongValue + w * output.strides[1].unsignedLongValue + c * output.strides[2].unsignedLongValue];
            return 1;
        } @catch (NSException *exception) { return 0; }
    }
}
