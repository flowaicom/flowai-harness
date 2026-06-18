export interface FeatureConfig {
  readonly apiBaseUrl: string;
  readonly routeBase: string;
  readonly storagePrefix: string;
}

export interface ChatConfig extends FeatureConfig {}

export interface EvalConfig extends FeatureConfig {
  readonly jobsEnabled: boolean;
}

export interface TestConfig extends FeatureConfig {}

export interface ConnectConfig extends FeatureConfig {
  readonly jobsEnabled: boolean;
}
