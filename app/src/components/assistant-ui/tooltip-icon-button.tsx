'use client';

import { cn } from '@/components/assistant-ui/lib/utils';
import { Button } from '@/components/assistant-ui/ui/button';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/assistant-ui/ui/tooltip';
import { type ComponentPropsWithRef, forwardRef } from 'react';

export type TooltipIconButtonProps = ComponentPropsWithRef<typeof Button> & {
  tooltip: string;
  side?: 'top' | 'bottom' | 'left' | 'right';
};

export const TooltipIconButton = forwardRef<HTMLButtonElement, TooltipIconButtonProps>(
  ({ children, tooltip, side = 'bottom', className, ...rest }, ref) => {
    return (
      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                variant="ghost"
                size="icon"
                {...rest}
                className={cn('aui-button-icon size-6 p-1 active:scale-90', className)}
                ref={ref}
              />
            }>
            {children}
            <span className="aui-sr-only sr-only">{tooltip}</span>
          </TooltipTrigger>
          <TooltipContent side={side}>{tooltip}</TooltipContent>
        </Tooltip>
      </TooltipProvider>
    );
  }
);

TooltipIconButton.displayName = 'TooltipIconButton';
