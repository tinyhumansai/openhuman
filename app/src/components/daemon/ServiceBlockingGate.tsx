interface ServiceBlockingGateProps {
  children: React.ReactNode;
}

const ServiceBlockingGate = ({ children }: ServiceBlockingGateProps) => {
  return <>{children}</>;
};

export default ServiceBlockingGate;
