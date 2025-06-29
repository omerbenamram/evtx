// Comprehensive logging system
export enum LogLevel {
  DEBUG = 0,
  INFO = 1,
  WARN = 2,
  ERROR = 3
}

export interface LogEntry {
  timestamp: Date;
  level: LogLevel;
  message: string;
  context?: any;
}

class Logger {
  private static instance: Logger;
  private logLevel: LogLevel = LogLevel.INFO;
  private logs: LogEntry[] = [];
  private maxLogs = 1000;
  private listeners: ((log: LogEntry) => void)[] = [];

  private constructor() {
    // Set log level from localStorage or default
    const savedLevel = localStorage.getItem('evtx-viewer-log-level');
    if (savedLevel) {
      this.logLevel = parseInt(savedLevel, 10);
    }
  }

  static getInstance(): Logger {
    if (!Logger.instance) {
      Logger.instance = new Logger();
    }
    return Logger.instance;
  }

  setLogLevel(level: LogLevel): void {
    this.logLevel = level;
    localStorage.setItem('evtx-viewer-log-level', level.toString());
  }

  getLogLevel(): LogLevel {
    return this.logLevel;
  }

  getLogs(): LogEntry[] {
    return [...this.logs];
  }

  clearLogs(): void {
    this.logs = [];
  }

  subscribe(listener: (log: LogEntry) => void): () => void {
    this.listeners.push(listener);
    return () => {
      this.listeners = this.listeners.filter(l => l !== listener);
    };
  }

  private log(level: LogLevel, message: string, context?: any): void {
    if (level < this.logLevel) return;

    const entry: LogEntry = {
      timestamp: new Date(),
      level,
      message,
      context
    };

    // Add to internal log buffer
    this.logs.push(entry);
    if (this.logs.length > this.maxLogs) {
      this.logs.shift();
    }

    // Notify listeners
    this.listeners.forEach(listener => listener(entry));

    // Console output
    const logMethod = level === LogLevel.ERROR ? 'error' : 
                     level === LogLevel.WARN ? 'warn' : 
                     level === LogLevel.INFO ? 'info' : 'log';
    
    const prefix = `[${LogLevel[level]}] ${entry.timestamp.toISOString()}`;
    if (context) {
      console[logMethod](prefix, message, context);
    } else {
      console[logMethod](prefix, message);
    }
  }

  debug(message: string, context?: any): void {
    this.log(LogLevel.DEBUG, message, context);
  }

  info(message: string, context?: any): void {
    this.log(LogLevel.INFO, message, context);
  }

  warn(message: string, context?: any): void {
    this.log(LogLevel.WARN, message, context);
  }

  error(message: string, context?: any): void {
    this.log(LogLevel.ERROR, message, context);
  }
}

export const logger = Logger.getInstance();