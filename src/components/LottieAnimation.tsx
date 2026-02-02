import { useLottie } from 'lottie-react';
import { useEffect, useState } from 'react';

interface LottieAnimationProps {
  src: string;
  className?: string;
  height?: number;
  width?: number;
}

const LottieAnimation = ({
  src,
  className = '',
  height = 200,
  width = 200,
}: LottieAnimationProps) => {
  const [animationData, setAnimationData] = useState<unknown>(null);

  useEffect(() => {
    fetch(src)
      .then(response => response.json())
      .then(data => setAnimationData(data))
      .catch(error => console.error('Failed to load Lottie animation:', error));
  }, [src]);

  const options = { animationData, loop: true, autoplay: true };

  const { View } = useLottie(options, { height, width });

  if (!animationData) {
    return <div className={className} style={{ height, width }} />;
  }

  return <div className={className}>{View}</div>;
};

export default LottieAnimation;
