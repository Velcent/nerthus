//! The IFC 2x3 schema as Assimp's machine-generated reader
//! (`IFCReaderGen_2x3`) converts it: the classes and their parents, the
//! arguments each class's `GenericFill` converts into which field slot
//! (with its UNSET and ISDERIVED checks and the argument count it
//! requires), and the converter each name in the schema has. Generated
//! from those sources; regenerate rather than edit.

#![allow(dead_code)]

/// The conversion `GenericConvert` makes for a field's type.
#[derive(Clone, Copy, Debug)]
pub(crate) enum FieldType {
    /// `PrimitiveDataType<double>`: a REAL (or NUMBER).
    Real,
    /// `PrimitiveDataType<int64_t>`: an INTEGER.
    Integer,
    /// `PrimitiveDataType<std::string>`: a STRING or an ENUMERATION.
    Text,
    /// `Lazy<T>`: an ENTITY reference, looked up (null when missing).
    Entity,
    /// `std::shared_ptr<const DataType>`: whatever the argument is.
    Select,
    /// `ListOf<T, min, max>`: a LIST of elements (the bounds only warn).
    List(&'static FieldType, u64, u64),
}

/// One argument a class's `GenericFill` converts.
#[derive(Debug)]
pub(crate) struct FieldFill {
    pub name: &'static str,
    pub slot: usize,
    pub field_type: FieldType,
    /// `Maybe<T>`: flagged valid once converted.
    pub maybe: bool,
    /// An ISDERIVED argument (`*`) leaves the field as it is.
    pub derived_check: bool,
    /// An UNSET argument (`$`) leaves the field as it is.
    pub unset_check: bool,
}

/// What a class's `GenericFill` does after its parent's: requires an
/// argument count, then converts its fields (classes the reader doesn't
/// use do neither).
#[derive(Debug)]
pub(crate) struct ClassFill {
    pub required: Option<usize>,
    pub fields: &'static [FieldFill],
}

/// A field of a class: the slot its `GenericFill` converts it into.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Slot {
    pub class: Class,
    pub index: usize,
}

/// The classes the reader constructs objects of.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Class {
    IfcRepresentationItem,
    IfcGeometricRepresentationItem,
    IfcCurve,
    IfcBoundedCurve,
    IfcCompositeCurve,
    Ifc2DCompositeCurve,
    IfcRoot,
    IfcObjectDefinition,
    IfcObject,
    IfcControl,
    IfcActionRequest,
    IfcActor,
    IfcTypeObject,
    IfcTypeProduct,
    IfcElementType,
    IfcDistributionElementType,
    IfcDistributionControlElementType,
    IfcActuatorType,
    IfcDistributionFlowElementType,
    IfcFlowControllerType,
    IfcAirTerminalBoxType,
    IfcFlowTerminalType,
    IfcAirTerminalType,
    IfcEnergyConversionDeviceType,
    IfcAirToAirHeatRecoveryType,
    IfcAlarmType,
    IfcDraughtingCallout,
    IfcDimensionCurveDirectedCallout,
    IfcAngularDimension,
    IfcProduct,
    IfcAnnotation,
    IfcStyledItem,
    IfcAnnotationOccurrence,
    IfcAnnotationCurveOccurrence,
    IfcAnnotationFillArea,
    IfcAnnotationFillAreaOccurrence,
    IfcAnnotationSurface,
    IfcAnnotationSurfaceOccurrence,
    IfcAnnotationSymbolOccurrence,
    IfcAnnotationTextOccurrence,
    IfcProfileDef,
    IfcArbitraryClosedProfileDef,
    IfcArbitraryOpenProfileDef,
    IfcArbitraryProfileDefWithVoids,
    IfcGroup,
    IfcAsset,
    IfcParameterizedProfileDef,
    IfcIShapeProfileDef,
    IfcAsymmetricIShapeProfileDef,
    IfcPlacement,
    IfcAxis1Placement,
    IfcAxis2Placement2D,
    IfcAxis2Placement3D,
    IfcBSplineCurve,
    IfcElement,
    IfcBuildingElement,
    IfcBeam,
    IfcBuildingElementType,
    IfcBeamType,
    IfcBezierCurve,
    IfcCsgPrimitive3D,
    IfcBlock,
    IfcBoilerType,
    IfcBooleanResult,
    IfcBooleanClippingResult,
    IfcSurface,
    IfcBoundedSurface,
    IfcBoundingBox,
    IfcHalfSpaceSolid,
    IfcBoxedHalfSpace,
    IfcSpatialStructureElement,
    IfcBuilding,
    IfcBuildingElementComponent,
    IfcBuildingElementPart,
    IfcBuildingElementProxy,
    IfcBuildingElementProxyType,
    IfcBuildingStorey,
    IfcCShapeProfileDef,
    IfcFlowFittingType,
    IfcCableCarrierFittingType,
    IfcFlowSegmentType,
    IfcCableCarrierSegmentType,
    IfcCableSegmentType,
    IfcPoint,
    IfcCartesianPoint,
    IfcCartesianTransformationOperator,
    IfcCartesianTransformationOperator2D,
    IfcCartesianTransformationOperator2DnonUniform,
    IfcCartesianTransformationOperator3D,
    IfcCartesianTransformationOperator3DnonUniform,
    IfcCenterLineProfileDef,
    IfcFeatureElement,
    IfcFeatureElementSubtraction,
    IfcEdgeFeature,
    IfcChamferEdgeFeature,
    IfcChillerType,
    IfcConic,
    IfcCircle,
    IfcCircleProfileDef,
    IfcCircleHollowProfileDef,
    IfcTopologicalRepresentationItem,
    IfcConnectedFaceSet,
    IfcClosedShell,
    IfcCoilType,
    IfcColourSpecification,
    IfcColourRgb,
    IfcColumn,
    IfcColumnType,
    IfcProperty,
    IfcComplexProperty,
    IfcCompositeCurveSegment,
    IfcCompositeProfileDef,
    IfcFlowMovingDeviceType,
    IfcCompressorType,
    IfcCondenserType,
    IfcCondition,
    IfcConditionCriterion,
    IfcResource,
    IfcConstructionResource,
    IfcConstructionEquipmentResource,
    IfcConstructionMaterialResource,
    IfcConstructionProductResource,
    IfcNamedUnit,
    IfcContextDependentUnit,
    IfcControllerType,
    IfcConversionBasedUnit,
    IfcCooledBeamType,
    IfcCoolingTowerType,
    IfcCostItem,
    IfcCostSchedule,
    IfcCovering,
    IfcCoveringType,
    IfcCraneRailAShapeProfileDef,
    IfcCraneRailFShapeProfileDef,
    IfcCrewResource,
    IfcSolidModel,
    IfcCsgSolid,
    IfcCurtainWall,
    IfcCurtainWallType,
    IfcCurveBoundedPlane,
    IfcPresentationStyle,
    IfcDamperType,
    IfcDefinedSymbol,
    IfcDerivedProfileDef,
    IfcDiameterDimension,
    IfcDimensionCurve,
    IfcTerminatorSymbol,
    IfcDimensionCurveTerminator,
    IfcDirection,
    IfcElementComponent,
    IfcDiscreteAccessory,
    IfcElementComponentType,
    IfcDiscreteAccessoryType,
    IfcDistributionElement,
    IfcDistributionFlowElement,
    IfcDistributionChamberElement,
    IfcDistributionChamberElementType,
    IfcDistributionControlElement,
    IfcPort,
    IfcDistributionPort,
    IfcDoor,
    IfcPropertyDefinition,
    IfcPropertySetDefinition,
    IfcDoorStyle,
    IfcDuctFittingType,
    IfcDuctSegmentType,
    IfcFlowTreatmentDeviceType,
    IfcDuctSilencerType,
    IfcEdge,
    IfcEdgeCurve,
    IfcLoop,
    IfcEdgeLoop,
    IfcElectricApplianceType,
    IfcFlowController,
    IfcElectricDistributionPoint,
    IfcFlowStorageDeviceType,
    IfcElectricFlowStorageDeviceType,
    IfcElectricGeneratorType,
    IfcElectricHeaterType,
    IfcElectricMotorType,
    IfcElectricTimeControlType,
    IfcSystem,
    IfcElectricalCircuit,
    IfcElectricalElement,
    IfcElementAssembly,
    IfcElementQuantity,
    IfcElementarySurface,
    IfcEllipse,
    IfcEllipseProfileDef,
    IfcEnergyConversionDevice,
    IfcEquipmentElement,
    IfcEquipmentStandard,
    IfcEvaporativeCoolerType,
    IfcEvaporatorType,
    IfcSweptAreaSolid,
    IfcExtrudedAreaSolid,
    IfcFace,
    IfcFaceBasedSurfaceModel,
    IfcFaceBound,
    IfcFaceOuterBound,
    IfcFaceSurface,
    IfcManifoldSolidBrep,
    IfcFacetedBrep,
    IfcFacetedBrepWithVoids,
    IfcFanType,
    IfcFastener,
    IfcFastenerType,
    IfcFeatureElementAddition,
    IfcFillAreaStyleHatching,
    IfcFillAreaStyleTileSymbolWithStyle,
    IfcFillAreaStyleTiles,
    IfcFilterType,
    IfcFireSuppressionTerminalType,
    IfcFlowFitting,
    IfcFlowInstrumentType,
    IfcFlowMeterType,
    IfcFlowMovingDevice,
    IfcFlowSegment,
    IfcFlowStorageDevice,
    IfcFlowTerminal,
    IfcFlowTreatmentDevice,
    IfcFooting,
    IfcFurnishingElement,
    IfcFurnishingElementType,
    IfcFurnitureStandard,
    IfcFurnitureType,
    IfcGasTerminalType,
    IfcGeometricSet,
    IfcGeometricCurveSet,
    IfcRepresentationContext,
    IfcGeometricRepresentationContext,
    IfcGeometricRepresentationSubContext,
    IfcGrid,
    IfcObjectPlacement,
    IfcGridPlacement,
    IfcHeatExchangerType,
    IfcHumidifierType,
    IfcInventory,
    IfcJunctionBoxType,
    IfcLShapeProfileDef,
    IfcLaborResource,
    IfcLampType,
    IfcLightFixtureType,
    IfcLightSource,
    IfcLightSourceAmbient,
    IfcLightSourceDirectional,
    IfcLightSourceGoniometric,
    IfcLightSourcePositional,
    IfcLightSourceSpot,
    IfcLine,
    IfcLinearDimension,
    IfcLocalPlacement,
    IfcMappedItem,
    IfcProductRepresentation,
    IfcMaterialDefinitionRepresentation,
    IfcMeasureWithUnit,
    IfcMechanicalFastener,
    IfcMechanicalFastenerType,
    IfcMember,
    IfcMemberType,
    IfcMotorConnectionType,
    IfcProcess,
    IfcTask,
    IfcMove,
    IfcOccupant,
    IfcOffsetCurve2D,
    IfcOffsetCurve3D,
    IfcOneDirectionRepeatFactor,
    IfcOpenShell,
    IfcOpeningElement,
    IfcOrderAction,
    IfcOrientedEdge,
    IfcOutletType,
    IfcPath,
    IfcPerformanceHistory,
    IfcPermit,
    IfcPile,
    IfcPipeFittingType,
    IfcPipeSegmentType,
    IfcPlanarExtent,
    IfcPlanarBox,
    IfcPlane,
    IfcPlate,
    IfcPlateType,
    IfcPointOnCurve,
    IfcPointOnSurface,
    IfcPolyLoop,
    IfcPolygonalBoundedHalfSpace,
    IfcPolyline,
    IfcPresentationStyleAssignment,
    IfcProcedure,
    IfcProductDefinitionShape,
    IfcProject,
    IfcProjectOrder,
    IfcProjectOrderRecord,
    IfcProjectionCurve,
    IfcProjectionElement,
    IfcSimpleProperty,
    IfcPropertyBoundedValue,
    IfcPropertyEnumeratedValue,
    IfcPropertyListValue,
    IfcPropertyReferenceValue,
    IfcPropertySet,
    IfcPropertySingleValue,
    IfcPropertyTableValue,
    IfcProtectiveDeviceType,
    IfcProxy,
    IfcPumpType,
    IfcRadiusDimension,
    IfcRailing,
    IfcRailingType,
    IfcRamp,
    IfcRampFlight,
    IfcRampFlightType,
    IfcRationalBezierCurve,
    IfcRectangleProfileDef,
    IfcRectangleHollowProfileDef,
    IfcRectangularPyramid,
    IfcRectangularTrimmedSurface,
    IfcReinforcingElement,
    IfcReinforcingBar,
    IfcReinforcingMesh,
    IfcRelationship,
    IfcRelDecomposes,
    IfcRelAggregates,
    IfcRelConnects,
    IfcRelContainedInSpatialStructure,
    IfcRelDefines,
    IfcRelDefinesByProperties,
    IfcRelFillsElement,
    IfcRelOverridesProperties,
    IfcRelVoidsElement,
    IfcRepresentation,
    IfcRepresentationMap,
    IfcRevolvedAreaSolid,
    IfcRightCircularCone,
    IfcRightCircularCylinder,
    IfcRoof,
    IfcRoundedEdgeFeature,
    IfcRoundedRectangleProfileDef,
    IfcSIUnit,
    IfcSanitaryTerminalType,
    IfcScheduleTimeControl,
    IfcSectionedSpine,
    IfcSensorType,
    IfcServiceLife,
    IfcShapeModel,
    IfcShapeRepresentation,
    IfcShellBasedSurfaceModel,
    IfcSite,
    IfcSlab,
    IfcSlabType,
    IfcSpace,
    IfcSpaceHeaterType,
    IfcSpaceProgram,
    IfcSpatialStructureElementType,
    IfcSpaceType,
    IfcSphere,
    IfcStackTerminalType,
    IfcStair,
    IfcStairFlight,
    IfcStairFlightType,
    IfcStructuralActivity,
    IfcStructuralAction,
    IfcStructuralAnalysisModel,
    IfcStructuralItem,
    IfcStructuralConnection,
    IfcStructuralCurveConnection,
    IfcStructuralMember,
    IfcStructuralCurveMember,
    IfcStructuralCurveMemberVarying,
    IfcStructuralLinearAction,
    IfcStructuralLinearActionVarying,
    IfcStructuralLoadGroup,
    IfcStructuralPlanarAction,
    IfcStructuralPlanarActionVarying,
    IfcStructuralPointAction,
    IfcStructuralPointConnection,
    IfcStructuralReaction,
    IfcStructuralPointReaction,
    IfcStructuralResultGroup,
    IfcStructuralSurfaceConnection,
    IfcStructuralSurfaceMember,
    IfcStructuralSurfaceMemberVarying,
    IfcStructuredDimensionCallout,
    IfcStyleModel,
    IfcStyledRepresentation,
    IfcSubContractResource,
    IfcSubedge,
    IfcSurfaceCurveSweptAreaSolid,
    IfcSweptSurface,
    IfcSurfaceOfLinearExtrusion,
    IfcSurfaceOfRevolution,
    IfcSurfaceStyle,
    IfcSurfaceStyleShading,
    IfcSurfaceStyleRendering,
    IfcSurfaceStyleWithTextures,
    IfcSweptDiskSolid,
    IfcSwitchingDeviceType,
    IfcSystemFurnitureElementType,
    IfcTShapeProfileDef,
    IfcTankType,
    IfcTendon,
    IfcTendonAnchor,
    IfcTextLiteral,
    IfcTextLiteralWithExtent,
    IfcTimeSeriesSchedule,
    IfcTopologyRepresentation,
    IfcTransformerType,
    IfcTransportElement,
    IfcTransportElementType,
    IfcTrapeziumProfileDef,
    IfcTrimmedCurve,
    IfcTubeBundleType,
    IfcTwoDirectionRepeatFactor,
    IfcUShapeProfileDef,
    IfcUnitAssignment,
    IfcUnitaryEquipmentType,
    IfcValveType,
    IfcVector,
    IfcVertex,
    IfcVertexLoop,
    IfcVertexPoint,
    IfcVibrationIsolatorType,
    IfcVirtualElement,
    IfcWall,
    IfcWallStandardCase,
    IfcWallType,
    IfcWasteTerminalType,
    IfcWindow,
    IfcWindowStyle,
    IfcWorkControl,
    IfcWorkPlan,
    IfcWorkSchedule,
    IfcZShapeProfileDef,
    IfcZone,
}

impl Class {
    /// The class it derives from.
    pub fn parent(self) -> Option<Class> {
        match self {
            Class::IfcRepresentationItem => None,
            Class::IfcGeometricRepresentationItem => Some(Class::IfcRepresentationItem),
            Class::IfcCurve => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcBoundedCurve => Some(Class::IfcCurve),
            Class::IfcCompositeCurve => Some(Class::IfcBoundedCurve),
            Class::Ifc2DCompositeCurve => Some(Class::IfcCompositeCurve),
            Class::IfcRoot => None,
            Class::IfcObjectDefinition => Some(Class::IfcRoot),
            Class::IfcObject => Some(Class::IfcObjectDefinition),
            Class::IfcControl => Some(Class::IfcObject),
            Class::IfcActionRequest => Some(Class::IfcControl),
            Class::IfcActor => Some(Class::IfcObject),
            Class::IfcTypeObject => Some(Class::IfcObjectDefinition),
            Class::IfcTypeProduct => Some(Class::IfcTypeObject),
            Class::IfcElementType => Some(Class::IfcTypeProduct),
            Class::IfcDistributionElementType => Some(Class::IfcElementType),
            Class::IfcDistributionControlElementType => Some(Class::IfcDistributionElementType),
            Class::IfcActuatorType => Some(Class::IfcDistributionControlElementType),
            Class::IfcDistributionFlowElementType => Some(Class::IfcDistributionElementType),
            Class::IfcFlowControllerType => Some(Class::IfcDistributionFlowElementType),
            Class::IfcAirTerminalBoxType => Some(Class::IfcFlowControllerType),
            Class::IfcFlowTerminalType => Some(Class::IfcDistributionFlowElementType),
            Class::IfcAirTerminalType => Some(Class::IfcFlowTerminalType),
            Class::IfcEnergyConversionDeviceType => Some(Class::IfcDistributionFlowElementType),
            Class::IfcAirToAirHeatRecoveryType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcAlarmType => Some(Class::IfcDistributionControlElementType),
            Class::IfcDraughtingCallout => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcDimensionCurveDirectedCallout => Some(Class::IfcDraughtingCallout),
            Class::IfcAngularDimension => Some(Class::IfcDimensionCurveDirectedCallout),
            Class::IfcProduct => Some(Class::IfcObject),
            Class::IfcAnnotation => Some(Class::IfcProduct),
            Class::IfcStyledItem => Some(Class::IfcRepresentationItem),
            Class::IfcAnnotationOccurrence => Some(Class::IfcStyledItem),
            Class::IfcAnnotationCurveOccurrence => Some(Class::IfcAnnotationOccurrence),
            Class::IfcAnnotationFillArea => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcAnnotationFillAreaOccurrence => Some(Class::IfcAnnotationOccurrence),
            Class::IfcAnnotationSurface => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcAnnotationSurfaceOccurrence => Some(Class::IfcAnnotationOccurrence),
            Class::IfcAnnotationSymbolOccurrence => Some(Class::IfcAnnotationOccurrence),
            Class::IfcAnnotationTextOccurrence => Some(Class::IfcAnnotationOccurrence),
            Class::IfcProfileDef => None,
            Class::IfcArbitraryClosedProfileDef => Some(Class::IfcProfileDef),
            Class::IfcArbitraryOpenProfileDef => Some(Class::IfcProfileDef),
            Class::IfcArbitraryProfileDefWithVoids => Some(Class::IfcArbitraryClosedProfileDef),
            Class::IfcGroup => Some(Class::IfcObject),
            Class::IfcAsset => Some(Class::IfcGroup),
            Class::IfcParameterizedProfileDef => Some(Class::IfcProfileDef),
            Class::IfcIShapeProfileDef => Some(Class::IfcParameterizedProfileDef),
            Class::IfcAsymmetricIShapeProfileDef => Some(Class::IfcIShapeProfileDef),
            Class::IfcPlacement => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcAxis1Placement => Some(Class::IfcPlacement),
            Class::IfcAxis2Placement2D => Some(Class::IfcPlacement),
            Class::IfcAxis2Placement3D => Some(Class::IfcPlacement),
            Class::IfcBSplineCurve => Some(Class::IfcBoundedCurve),
            Class::IfcElement => Some(Class::IfcProduct),
            Class::IfcBuildingElement => Some(Class::IfcElement),
            Class::IfcBeam => Some(Class::IfcBuildingElement),
            Class::IfcBuildingElementType => Some(Class::IfcElementType),
            Class::IfcBeamType => Some(Class::IfcBuildingElementType),
            Class::IfcBezierCurve => Some(Class::IfcBSplineCurve),
            Class::IfcCsgPrimitive3D => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcBlock => Some(Class::IfcCsgPrimitive3D),
            Class::IfcBoilerType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcBooleanResult => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcBooleanClippingResult => Some(Class::IfcBooleanResult),
            Class::IfcSurface => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcBoundedSurface => Some(Class::IfcSurface),
            Class::IfcBoundingBox => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcHalfSpaceSolid => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcBoxedHalfSpace => Some(Class::IfcHalfSpaceSolid),
            Class::IfcSpatialStructureElement => Some(Class::IfcProduct),
            Class::IfcBuilding => Some(Class::IfcSpatialStructureElement),
            Class::IfcBuildingElementComponent => Some(Class::IfcBuildingElement),
            Class::IfcBuildingElementPart => Some(Class::IfcBuildingElementComponent),
            Class::IfcBuildingElementProxy => Some(Class::IfcBuildingElement),
            Class::IfcBuildingElementProxyType => Some(Class::IfcBuildingElementType),
            Class::IfcBuildingStorey => Some(Class::IfcSpatialStructureElement),
            Class::IfcCShapeProfileDef => Some(Class::IfcParameterizedProfileDef),
            Class::IfcFlowFittingType => Some(Class::IfcDistributionFlowElementType),
            Class::IfcCableCarrierFittingType => Some(Class::IfcFlowFittingType),
            Class::IfcFlowSegmentType => Some(Class::IfcDistributionFlowElementType),
            Class::IfcCableCarrierSegmentType => Some(Class::IfcFlowSegmentType),
            Class::IfcCableSegmentType => Some(Class::IfcFlowSegmentType),
            Class::IfcPoint => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcCartesianPoint => Some(Class::IfcPoint),
            Class::IfcCartesianTransformationOperator => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcCartesianTransformationOperator2D => Some(Class::IfcCartesianTransformationOperator),
            Class::IfcCartesianTransformationOperator2DnonUniform => Some(Class::IfcCartesianTransformationOperator2D),
            Class::IfcCartesianTransformationOperator3D => Some(Class::IfcCartesianTransformationOperator),
            Class::IfcCartesianTransformationOperator3DnonUniform => Some(Class::IfcCartesianTransformationOperator3D),
            Class::IfcCenterLineProfileDef => Some(Class::IfcArbitraryOpenProfileDef),
            Class::IfcFeatureElement => Some(Class::IfcElement),
            Class::IfcFeatureElementSubtraction => Some(Class::IfcFeatureElement),
            Class::IfcEdgeFeature => Some(Class::IfcFeatureElementSubtraction),
            Class::IfcChamferEdgeFeature => Some(Class::IfcEdgeFeature),
            Class::IfcChillerType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcConic => Some(Class::IfcCurve),
            Class::IfcCircle => Some(Class::IfcConic),
            Class::IfcCircleProfileDef => Some(Class::IfcParameterizedProfileDef),
            Class::IfcCircleHollowProfileDef => Some(Class::IfcCircleProfileDef),
            Class::IfcTopologicalRepresentationItem => Some(Class::IfcRepresentationItem),
            Class::IfcConnectedFaceSet => Some(Class::IfcTopologicalRepresentationItem),
            Class::IfcClosedShell => Some(Class::IfcConnectedFaceSet),
            Class::IfcCoilType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcColourSpecification => None,
            Class::IfcColourRgb => Some(Class::IfcColourSpecification),
            Class::IfcColumn => Some(Class::IfcBuildingElement),
            Class::IfcColumnType => Some(Class::IfcBuildingElementType),
            Class::IfcProperty => None,
            Class::IfcComplexProperty => Some(Class::IfcProperty),
            Class::IfcCompositeCurveSegment => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcCompositeProfileDef => Some(Class::IfcProfileDef),
            Class::IfcFlowMovingDeviceType => Some(Class::IfcDistributionFlowElementType),
            Class::IfcCompressorType => Some(Class::IfcFlowMovingDeviceType),
            Class::IfcCondenserType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcCondition => Some(Class::IfcGroup),
            Class::IfcConditionCriterion => Some(Class::IfcControl),
            Class::IfcResource => Some(Class::IfcObject),
            Class::IfcConstructionResource => Some(Class::IfcResource),
            Class::IfcConstructionEquipmentResource => Some(Class::IfcConstructionResource),
            Class::IfcConstructionMaterialResource => Some(Class::IfcConstructionResource),
            Class::IfcConstructionProductResource => Some(Class::IfcConstructionResource),
            Class::IfcNamedUnit => None,
            Class::IfcContextDependentUnit => Some(Class::IfcNamedUnit),
            Class::IfcControllerType => Some(Class::IfcDistributionControlElementType),
            Class::IfcConversionBasedUnit => Some(Class::IfcNamedUnit),
            Class::IfcCooledBeamType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcCoolingTowerType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcCostItem => Some(Class::IfcControl),
            Class::IfcCostSchedule => Some(Class::IfcControl),
            Class::IfcCovering => Some(Class::IfcBuildingElement),
            Class::IfcCoveringType => Some(Class::IfcBuildingElementType),
            Class::IfcCraneRailAShapeProfileDef => Some(Class::IfcParameterizedProfileDef),
            Class::IfcCraneRailFShapeProfileDef => Some(Class::IfcParameterizedProfileDef),
            Class::IfcCrewResource => Some(Class::IfcConstructionResource),
            Class::IfcSolidModel => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcCsgSolid => Some(Class::IfcSolidModel),
            Class::IfcCurtainWall => Some(Class::IfcBuildingElement),
            Class::IfcCurtainWallType => Some(Class::IfcBuildingElementType),
            Class::IfcCurveBoundedPlane => Some(Class::IfcBoundedSurface),
            Class::IfcPresentationStyle => None,
            Class::IfcDamperType => Some(Class::IfcFlowControllerType),
            Class::IfcDefinedSymbol => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcDerivedProfileDef => Some(Class::IfcProfileDef),
            Class::IfcDiameterDimension => Some(Class::IfcDimensionCurveDirectedCallout),
            Class::IfcDimensionCurve => Some(Class::IfcAnnotationCurveOccurrence),
            Class::IfcTerminatorSymbol => Some(Class::IfcAnnotationSymbolOccurrence),
            Class::IfcDimensionCurveTerminator => Some(Class::IfcTerminatorSymbol),
            Class::IfcDirection => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcElementComponent => Some(Class::IfcElement),
            Class::IfcDiscreteAccessory => Some(Class::IfcElementComponent),
            Class::IfcElementComponentType => Some(Class::IfcElementType),
            Class::IfcDiscreteAccessoryType => Some(Class::IfcElementComponentType),
            Class::IfcDistributionElement => Some(Class::IfcElement),
            Class::IfcDistributionFlowElement => Some(Class::IfcDistributionElement),
            Class::IfcDistributionChamberElement => Some(Class::IfcDistributionFlowElement),
            Class::IfcDistributionChamberElementType => Some(Class::IfcDistributionFlowElementType),
            Class::IfcDistributionControlElement => Some(Class::IfcDistributionElement),
            Class::IfcPort => Some(Class::IfcProduct),
            Class::IfcDistributionPort => Some(Class::IfcPort),
            Class::IfcDoor => Some(Class::IfcBuildingElement),
            Class::IfcPropertyDefinition => Some(Class::IfcRoot),
            Class::IfcPropertySetDefinition => Some(Class::IfcPropertyDefinition),
            Class::IfcDoorStyle => Some(Class::IfcTypeProduct),
            Class::IfcDuctFittingType => Some(Class::IfcFlowFittingType),
            Class::IfcDuctSegmentType => Some(Class::IfcFlowSegmentType),
            Class::IfcFlowTreatmentDeviceType => Some(Class::IfcDistributionFlowElementType),
            Class::IfcDuctSilencerType => Some(Class::IfcFlowTreatmentDeviceType),
            Class::IfcEdge => Some(Class::IfcTopologicalRepresentationItem),
            Class::IfcEdgeCurve => Some(Class::IfcEdge),
            Class::IfcLoop => Some(Class::IfcTopologicalRepresentationItem),
            Class::IfcEdgeLoop => Some(Class::IfcLoop),
            Class::IfcElectricApplianceType => Some(Class::IfcFlowTerminalType),
            Class::IfcFlowController => Some(Class::IfcDistributionFlowElement),
            Class::IfcElectricDistributionPoint => Some(Class::IfcFlowController),
            Class::IfcFlowStorageDeviceType => Some(Class::IfcDistributionFlowElementType),
            Class::IfcElectricFlowStorageDeviceType => Some(Class::IfcFlowStorageDeviceType),
            Class::IfcElectricGeneratorType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcElectricHeaterType => Some(Class::IfcFlowTerminalType),
            Class::IfcElectricMotorType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcElectricTimeControlType => Some(Class::IfcFlowControllerType),
            Class::IfcSystem => Some(Class::IfcGroup),
            Class::IfcElectricalCircuit => Some(Class::IfcSystem),
            Class::IfcElectricalElement => Some(Class::IfcElement),
            Class::IfcElementAssembly => Some(Class::IfcElement),
            Class::IfcElementQuantity => Some(Class::IfcPropertySetDefinition),
            Class::IfcElementarySurface => Some(Class::IfcSurface),
            Class::IfcEllipse => Some(Class::IfcConic),
            Class::IfcEllipseProfileDef => Some(Class::IfcParameterizedProfileDef),
            Class::IfcEnergyConversionDevice => Some(Class::IfcDistributionFlowElement),
            Class::IfcEquipmentElement => Some(Class::IfcElement),
            Class::IfcEquipmentStandard => Some(Class::IfcControl),
            Class::IfcEvaporativeCoolerType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcEvaporatorType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcSweptAreaSolid => Some(Class::IfcSolidModel),
            Class::IfcExtrudedAreaSolid => Some(Class::IfcSweptAreaSolid),
            Class::IfcFace => Some(Class::IfcTopologicalRepresentationItem),
            Class::IfcFaceBasedSurfaceModel => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcFaceBound => Some(Class::IfcTopologicalRepresentationItem),
            Class::IfcFaceOuterBound => Some(Class::IfcFaceBound),
            Class::IfcFaceSurface => Some(Class::IfcFace),
            Class::IfcManifoldSolidBrep => Some(Class::IfcSolidModel),
            Class::IfcFacetedBrep => Some(Class::IfcManifoldSolidBrep),
            Class::IfcFacetedBrepWithVoids => Some(Class::IfcManifoldSolidBrep),
            Class::IfcFanType => Some(Class::IfcFlowMovingDeviceType),
            Class::IfcFastener => Some(Class::IfcElementComponent),
            Class::IfcFastenerType => Some(Class::IfcElementComponentType),
            Class::IfcFeatureElementAddition => Some(Class::IfcFeatureElement),
            Class::IfcFillAreaStyleHatching => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcFillAreaStyleTileSymbolWithStyle => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcFillAreaStyleTiles => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcFilterType => Some(Class::IfcFlowTreatmentDeviceType),
            Class::IfcFireSuppressionTerminalType => Some(Class::IfcFlowTerminalType),
            Class::IfcFlowFitting => Some(Class::IfcDistributionFlowElement),
            Class::IfcFlowInstrumentType => Some(Class::IfcDistributionControlElementType),
            Class::IfcFlowMeterType => Some(Class::IfcFlowControllerType),
            Class::IfcFlowMovingDevice => Some(Class::IfcDistributionFlowElement),
            Class::IfcFlowSegment => Some(Class::IfcDistributionFlowElement),
            Class::IfcFlowStorageDevice => Some(Class::IfcDistributionFlowElement),
            Class::IfcFlowTerminal => Some(Class::IfcDistributionFlowElement),
            Class::IfcFlowTreatmentDevice => Some(Class::IfcDistributionFlowElement),
            Class::IfcFooting => Some(Class::IfcBuildingElement),
            Class::IfcFurnishingElement => Some(Class::IfcElement),
            Class::IfcFurnishingElementType => Some(Class::IfcElementType),
            Class::IfcFurnitureStandard => Some(Class::IfcControl),
            Class::IfcFurnitureType => Some(Class::IfcFurnishingElementType),
            Class::IfcGasTerminalType => Some(Class::IfcFlowTerminalType),
            Class::IfcGeometricSet => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcGeometricCurveSet => Some(Class::IfcGeometricSet),
            Class::IfcRepresentationContext => None,
            Class::IfcGeometricRepresentationContext => Some(Class::IfcRepresentationContext),
            Class::IfcGeometricRepresentationSubContext => Some(Class::IfcGeometricRepresentationContext),
            Class::IfcGrid => Some(Class::IfcProduct),
            Class::IfcObjectPlacement => None,
            Class::IfcGridPlacement => Some(Class::IfcObjectPlacement),
            Class::IfcHeatExchangerType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcHumidifierType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcInventory => Some(Class::IfcGroup),
            Class::IfcJunctionBoxType => Some(Class::IfcFlowFittingType),
            Class::IfcLShapeProfileDef => Some(Class::IfcParameterizedProfileDef),
            Class::IfcLaborResource => Some(Class::IfcConstructionResource),
            Class::IfcLampType => Some(Class::IfcFlowTerminalType),
            Class::IfcLightFixtureType => Some(Class::IfcFlowTerminalType),
            Class::IfcLightSource => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcLightSourceAmbient => Some(Class::IfcLightSource),
            Class::IfcLightSourceDirectional => Some(Class::IfcLightSource),
            Class::IfcLightSourceGoniometric => Some(Class::IfcLightSource),
            Class::IfcLightSourcePositional => Some(Class::IfcLightSource),
            Class::IfcLightSourceSpot => Some(Class::IfcLightSourcePositional),
            Class::IfcLine => Some(Class::IfcCurve),
            Class::IfcLinearDimension => Some(Class::IfcDimensionCurveDirectedCallout),
            Class::IfcLocalPlacement => Some(Class::IfcObjectPlacement),
            Class::IfcMappedItem => Some(Class::IfcRepresentationItem),
            Class::IfcProductRepresentation => None,
            Class::IfcMaterialDefinitionRepresentation => Some(Class::IfcProductRepresentation),
            Class::IfcMeasureWithUnit => None,
            Class::IfcMechanicalFastener => Some(Class::IfcFastener),
            Class::IfcMechanicalFastenerType => Some(Class::IfcFastenerType),
            Class::IfcMember => Some(Class::IfcBuildingElement),
            Class::IfcMemberType => Some(Class::IfcBuildingElementType),
            Class::IfcMotorConnectionType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcProcess => Some(Class::IfcObject),
            Class::IfcTask => Some(Class::IfcProcess),
            Class::IfcMove => Some(Class::IfcTask),
            Class::IfcOccupant => Some(Class::IfcActor),
            Class::IfcOffsetCurve2D => Some(Class::IfcCurve),
            Class::IfcOffsetCurve3D => Some(Class::IfcCurve),
            Class::IfcOneDirectionRepeatFactor => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcOpenShell => Some(Class::IfcConnectedFaceSet),
            Class::IfcOpeningElement => Some(Class::IfcFeatureElementSubtraction),
            Class::IfcOrderAction => Some(Class::IfcTask),
            Class::IfcOrientedEdge => Some(Class::IfcEdge),
            Class::IfcOutletType => Some(Class::IfcFlowTerminalType),
            Class::IfcPath => Some(Class::IfcTopologicalRepresentationItem),
            Class::IfcPerformanceHistory => Some(Class::IfcControl),
            Class::IfcPermit => Some(Class::IfcControl),
            Class::IfcPile => Some(Class::IfcBuildingElement),
            Class::IfcPipeFittingType => Some(Class::IfcFlowFittingType),
            Class::IfcPipeSegmentType => Some(Class::IfcFlowSegmentType),
            Class::IfcPlanarExtent => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcPlanarBox => Some(Class::IfcPlanarExtent),
            Class::IfcPlane => Some(Class::IfcElementarySurface),
            Class::IfcPlate => Some(Class::IfcBuildingElement),
            Class::IfcPlateType => Some(Class::IfcBuildingElementType),
            Class::IfcPointOnCurve => Some(Class::IfcPoint),
            Class::IfcPointOnSurface => Some(Class::IfcPoint),
            Class::IfcPolyLoop => Some(Class::IfcLoop),
            Class::IfcPolygonalBoundedHalfSpace => Some(Class::IfcHalfSpaceSolid),
            Class::IfcPolyline => Some(Class::IfcBoundedCurve),
            Class::IfcPresentationStyleAssignment => None,
            Class::IfcProcedure => Some(Class::IfcProcess),
            Class::IfcProductDefinitionShape => Some(Class::IfcProductRepresentation),
            Class::IfcProject => Some(Class::IfcObject),
            Class::IfcProjectOrder => Some(Class::IfcControl),
            Class::IfcProjectOrderRecord => Some(Class::IfcControl),
            Class::IfcProjectionCurve => Some(Class::IfcAnnotationCurveOccurrence),
            Class::IfcProjectionElement => Some(Class::IfcFeatureElementAddition),
            Class::IfcSimpleProperty => Some(Class::IfcProperty),
            Class::IfcPropertyBoundedValue => Some(Class::IfcSimpleProperty),
            Class::IfcPropertyEnumeratedValue => Some(Class::IfcSimpleProperty),
            Class::IfcPropertyListValue => Some(Class::IfcSimpleProperty),
            Class::IfcPropertyReferenceValue => Some(Class::IfcSimpleProperty),
            Class::IfcPropertySet => Some(Class::IfcPropertySetDefinition),
            Class::IfcPropertySingleValue => Some(Class::IfcSimpleProperty),
            Class::IfcPropertyTableValue => Some(Class::IfcSimpleProperty),
            Class::IfcProtectiveDeviceType => Some(Class::IfcFlowControllerType),
            Class::IfcProxy => Some(Class::IfcProduct),
            Class::IfcPumpType => Some(Class::IfcFlowMovingDeviceType),
            Class::IfcRadiusDimension => Some(Class::IfcDimensionCurveDirectedCallout),
            Class::IfcRailing => Some(Class::IfcBuildingElement),
            Class::IfcRailingType => Some(Class::IfcBuildingElementType),
            Class::IfcRamp => Some(Class::IfcBuildingElement),
            Class::IfcRampFlight => Some(Class::IfcBuildingElement),
            Class::IfcRampFlightType => Some(Class::IfcBuildingElementType),
            Class::IfcRationalBezierCurve => Some(Class::IfcBezierCurve),
            Class::IfcRectangleProfileDef => Some(Class::IfcParameterizedProfileDef),
            Class::IfcRectangleHollowProfileDef => Some(Class::IfcRectangleProfileDef),
            Class::IfcRectangularPyramid => Some(Class::IfcCsgPrimitive3D),
            Class::IfcRectangularTrimmedSurface => Some(Class::IfcBoundedSurface),
            Class::IfcReinforcingElement => Some(Class::IfcBuildingElementComponent),
            Class::IfcReinforcingBar => Some(Class::IfcReinforcingElement),
            Class::IfcReinforcingMesh => Some(Class::IfcReinforcingElement),
            Class::IfcRelationship => Some(Class::IfcRoot),
            Class::IfcRelDecomposes => Some(Class::IfcRelationship),
            Class::IfcRelAggregates => Some(Class::IfcRelDecomposes),
            Class::IfcRelConnects => Some(Class::IfcRelationship),
            Class::IfcRelContainedInSpatialStructure => Some(Class::IfcRelConnects),
            Class::IfcRelDefines => Some(Class::IfcRelationship),
            Class::IfcRelDefinesByProperties => Some(Class::IfcRelDefines),
            Class::IfcRelFillsElement => Some(Class::IfcRelConnects),
            Class::IfcRelOverridesProperties => Some(Class::IfcRelDefinesByProperties),
            Class::IfcRelVoidsElement => Some(Class::IfcRelConnects),
            Class::IfcRepresentation => None,
            Class::IfcRepresentationMap => None,
            Class::IfcRevolvedAreaSolid => Some(Class::IfcSweptAreaSolid),
            Class::IfcRightCircularCone => Some(Class::IfcCsgPrimitive3D),
            Class::IfcRightCircularCylinder => Some(Class::IfcCsgPrimitive3D),
            Class::IfcRoof => Some(Class::IfcBuildingElement),
            Class::IfcRoundedEdgeFeature => Some(Class::IfcEdgeFeature),
            Class::IfcRoundedRectangleProfileDef => Some(Class::IfcRectangleProfileDef),
            Class::IfcSIUnit => Some(Class::IfcNamedUnit),
            Class::IfcSanitaryTerminalType => Some(Class::IfcFlowTerminalType),
            Class::IfcScheduleTimeControl => Some(Class::IfcControl),
            Class::IfcSectionedSpine => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcSensorType => Some(Class::IfcDistributionControlElementType),
            Class::IfcServiceLife => Some(Class::IfcControl),
            Class::IfcShapeModel => Some(Class::IfcRepresentation),
            Class::IfcShapeRepresentation => Some(Class::IfcShapeModel),
            Class::IfcShellBasedSurfaceModel => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcSite => Some(Class::IfcSpatialStructureElement),
            Class::IfcSlab => Some(Class::IfcBuildingElement),
            Class::IfcSlabType => Some(Class::IfcBuildingElementType),
            Class::IfcSpace => Some(Class::IfcSpatialStructureElement),
            Class::IfcSpaceHeaterType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcSpaceProgram => Some(Class::IfcControl),
            Class::IfcSpatialStructureElementType => Some(Class::IfcElementType),
            Class::IfcSpaceType => Some(Class::IfcSpatialStructureElementType),
            Class::IfcSphere => Some(Class::IfcCsgPrimitive3D),
            Class::IfcStackTerminalType => Some(Class::IfcFlowTerminalType),
            Class::IfcStair => Some(Class::IfcBuildingElement),
            Class::IfcStairFlight => Some(Class::IfcBuildingElement),
            Class::IfcStairFlightType => Some(Class::IfcBuildingElementType),
            Class::IfcStructuralActivity => Some(Class::IfcProduct),
            Class::IfcStructuralAction => Some(Class::IfcStructuralActivity),
            Class::IfcStructuralAnalysisModel => Some(Class::IfcSystem),
            Class::IfcStructuralItem => Some(Class::IfcProduct),
            Class::IfcStructuralConnection => Some(Class::IfcStructuralItem),
            Class::IfcStructuralCurveConnection => Some(Class::IfcStructuralConnection),
            Class::IfcStructuralMember => Some(Class::IfcStructuralItem),
            Class::IfcStructuralCurveMember => Some(Class::IfcStructuralMember),
            Class::IfcStructuralCurveMemberVarying => Some(Class::IfcStructuralCurveMember),
            Class::IfcStructuralLinearAction => Some(Class::IfcStructuralAction),
            Class::IfcStructuralLinearActionVarying => Some(Class::IfcStructuralLinearAction),
            Class::IfcStructuralLoadGroup => Some(Class::IfcGroup),
            Class::IfcStructuralPlanarAction => Some(Class::IfcStructuralAction),
            Class::IfcStructuralPlanarActionVarying => Some(Class::IfcStructuralPlanarAction),
            Class::IfcStructuralPointAction => Some(Class::IfcStructuralAction),
            Class::IfcStructuralPointConnection => Some(Class::IfcStructuralConnection),
            Class::IfcStructuralReaction => Some(Class::IfcStructuralActivity),
            Class::IfcStructuralPointReaction => Some(Class::IfcStructuralReaction),
            Class::IfcStructuralResultGroup => Some(Class::IfcGroup),
            Class::IfcStructuralSurfaceConnection => Some(Class::IfcStructuralConnection),
            Class::IfcStructuralSurfaceMember => Some(Class::IfcStructuralMember),
            Class::IfcStructuralSurfaceMemberVarying => Some(Class::IfcStructuralSurfaceMember),
            Class::IfcStructuredDimensionCallout => Some(Class::IfcDraughtingCallout),
            Class::IfcStyleModel => Some(Class::IfcRepresentation),
            Class::IfcStyledRepresentation => Some(Class::IfcStyleModel),
            Class::IfcSubContractResource => Some(Class::IfcConstructionResource),
            Class::IfcSubedge => Some(Class::IfcEdge),
            Class::IfcSurfaceCurveSweptAreaSolid => Some(Class::IfcSweptAreaSolid),
            Class::IfcSweptSurface => Some(Class::IfcSurface),
            Class::IfcSurfaceOfLinearExtrusion => Some(Class::IfcSweptSurface),
            Class::IfcSurfaceOfRevolution => Some(Class::IfcSweptSurface),
            Class::IfcSurfaceStyle => Some(Class::IfcPresentationStyle),
            Class::IfcSurfaceStyleShading => None,
            Class::IfcSurfaceStyleRendering => Some(Class::IfcSurfaceStyleShading),
            Class::IfcSurfaceStyleWithTextures => None,
            Class::IfcSweptDiskSolid => Some(Class::IfcSolidModel),
            Class::IfcSwitchingDeviceType => Some(Class::IfcFlowControllerType),
            Class::IfcSystemFurnitureElementType => Some(Class::IfcFurnishingElementType),
            Class::IfcTShapeProfileDef => Some(Class::IfcParameterizedProfileDef),
            Class::IfcTankType => Some(Class::IfcFlowStorageDeviceType),
            Class::IfcTendon => Some(Class::IfcReinforcingElement),
            Class::IfcTendonAnchor => Some(Class::IfcReinforcingElement),
            Class::IfcTextLiteral => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcTextLiteralWithExtent => Some(Class::IfcTextLiteral),
            Class::IfcTimeSeriesSchedule => Some(Class::IfcControl),
            Class::IfcTopologyRepresentation => Some(Class::IfcShapeModel),
            Class::IfcTransformerType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcTransportElement => Some(Class::IfcElement),
            Class::IfcTransportElementType => Some(Class::IfcElementType),
            Class::IfcTrapeziumProfileDef => Some(Class::IfcParameterizedProfileDef),
            Class::IfcTrimmedCurve => Some(Class::IfcBoundedCurve),
            Class::IfcTubeBundleType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcTwoDirectionRepeatFactor => Some(Class::IfcOneDirectionRepeatFactor),
            Class::IfcUShapeProfileDef => Some(Class::IfcParameterizedProfileDef),
            Class::IfcUnitAssignment => None,
            Class::IfcUnitaryEquipmentType => Some(Class::IfcEnergyConversionDeviceType),
            Class::IfcValveType => Some(Class::IfcFlowControllerType),
            Class::IfcVector => Some(Class::IfcGeometricRepresentationItem),
            Class::IfcVertex => Some(Class::IfcTopologicalRepresentationItem),
            Class::IfcVertexLoop => Some(Class::IfcLoop),
            Class::IfcVertexPoint => Some(Class::IfcVertex),
            Class::IfcVibrationIsolatorType => Some(Class::IfcDiscreteAccessoryType),
            Class::IfcVirtualElement => Some(Class::IfcElement),
            Class::IfcWall => Some(Class::IfcBuildingElement),
            Class::IfcWallStandardCase => Some(Class::IfcWall),
            Class::IfcWallType => Some(Class::IfcBuildingElementType),
            Class::IfcWasteTerminalType => Some(Class::IfcFlowTerminalType),
            Class::IfcWindow => Some(Class::IfcBuildingElement),
            Class::IfcWindowStyle => Some(Class::IfcTypeProduct),
            Class::IfcWorkControl => Some(Class::IfcControl),
            Class::IfcWorkPlan => Some(Class::IfcWorkControl),
            Class::IfcWorkSchedule => Some(Class::IfcWorkControl),
            Class::IfcZShapeProfileDef => Some(Class::IfcParameterizedProfileDef),
            Class::IfcZone => Some(Class::IfcGroup),
        }
    }

    /// The C++ class name (`Object::GetClassName`).
    pub fn name(self) -> &'static str {
        match self {
            Class::IfcRepresentationItem => "IfcRepresentationItem",
            Class::IfcGeometricRepresentationItem => "IfcGeometricRepresentationItem",
            Class::IfcCurve => "IfcCurve",
            Class::IfcBoundedCurve => "IfcBoundedCurve",
            Class::IfcCompositeCurve => "IfcCompositeCurve",
            Class::Ifc2DCompositeCurve => "Ifc2DCompositeCurve",
            Class::IfcRoot => "IfcRoot",
            Class::IfcObjectDefinition => "IfcObjectDefinition",
            Class::IfcObject => "IfcObject",
            Class::IfcControl => "IfcControl",
            Class::IfcActionRequest => "IfcActionRequest",
            Class::IfcActor => "IfcActor",
            Class::IfcTypeObject => "IfcTypeObject",
            Class::IfcTypeProduct => "IfcTypeProduct",
            Class::IfcElementType => "IfcElementType",
            Class::IfcDistributionElementType => "IfcDistributionElementType",
            Class::IfcDistributionControlElementType => "IfcDistributionControlElementType",
            Class::IfcActuatorType => "IfcActuatorType",
            Class::IfcDistributionFlowElementType => "IfcDistributionFlowElementType",
            Class::IfcFlowControllerType => "IfcFlowControllerType",
            Class::IfcAirTerminalBoxType => "IfcAirTerminalBoxType",
            Class::IfcFlowTerminalType => "IfcFlowTerminalType",
            Class::IfcAirTerminalType => "IfcAirTerminalType",
            Class::IfcEnergyConversionDeviceType => "IfcEnergyConversionDeviceType",
            Class::IfcAirToAirHeatRecoveryType => "IfcAirToAirHeatRecoveryType",
            Class::IfcAlarmType => "IfcAlarmType",
            Class::IfcDraughtingCallout => "IfcDraughtingCallout",
            Class::IfcDimensionCurveDirectedCallout => "IfcDimensionCurveDirectedCallout",
            Class::IfcAngularDimension => "IfcAngularDimension",
            Class::IfcProduct => "IfcProduct",
            Class::IfcAnnotation => "IfcAnnotation",
            Class::IfcStyledItem => "IfcStyledItem",
            Class::IfcAnnotationOccurrence => "IfcAnnotationOccurrence",
            Class::IfcAnnotationCurveOccurrence => "IfcAnnotationCurveOccurrence",
            Class::IfcAnnotationFillArea => "IfcAnnotationFillArea",
            Class::IfcAnnotationFillAreaOccurrence => "IfcAnnotationFillAreaOccurrence",
            Class::IfcAnnotationSurface => "IfcAnnotationSurface",
            Class::IfcAnnotationSurfaceOccurrence => "IfcAnnotationSurfaceOccurrence",
            Class::IfcAnnotationSymbolOccurrence => "IfcAnnotationSymbolOccurrence",
            Class::IfcAnnotationTextOccurrence => "IfcAnnotationTextOccurrence",
            Class::IfcProfileDef => "IfcProfileDef",
            Class::IfcArbitraryClosedProfileDef => "IfcArbitraryClosedProfileDef",
            Class::IfcArbitraryOpenProfileDef => "IfcArbitraryOpenProfileDef",
            Class::IfcArbitraryProfileDefWithVoids => "IfcArbitraryProfileDefWithVoids",
            Class::IfcGroup => "IfcGroup",
            Class::IfcAsset => "IfcAsset",
            Class::IfcParameterizedProfileDef => "IfcParameterizedProfileDef",
            Class::IfcIShapeProfileDef => "IfcIShapeProfileDef",
            Class::IfcAsymmetricIShapeProfileDef => "IfcAsymmetricIShapeProfileDef",
            Class::IfcPlacement => "IfcPlacement",
            Class::IfcAxis1Placement => "IfcAxis1Placement",
            Class::IfcAxis2Placement2D => "IfcAxis2Placement2D",
            Class::IfcAxis2Placement3D => "IfcAxis2Placement3D",
            Class::IfcBSplineCurve => "IfcBSplineCurve",
            Class::IfcElement => "IfcElement",
            Class::IfcBuildingElement => "IfcBuildingElement",
            Class::IfcBeam => "IfcBeam",
            Class::IfcBuildingElementType => "IfcBuildingElementType",
            Class::IfcBeamType => "IfcBeamType",
            Class::IfcBezierCurve => "IfcBezierCurve",
            Class::IfcCsgPrimitive3D => "IfcCsgPrimitive3D",
            Class::IfcBlock => "IfcBlock",
            Class::IfcBoilerType => "IfcBoilerType",
            Class::IfcBooleanResult => "IfcBooleanResult",
            Class::IfcBooleanClippingResult => "IfcBooleanClippingResult",
            Class::IfcSurface => "IfcSurface",
            Class::IfcBoundedSurface => "IfcBoundedSurface",
            Class::IfcBoundingBox => "IfcBoundingBox",
            Class::IfcHalfSpaceSolid => "IfcHalfSpaceSolid",
            Class::IfcBoxedHalfSpace => "IfcBoxedHalfSpace",
            Class::IfcSpatialStructureElement => "IfcSpatialStructureElement",
            Class::IfcBuilding => "IfcBuilding",
            Class::IfcBuildingElementComponent => "IfcBuildingElementComponent",
            Class::IfcBuildingElementPart => "IfcBuildingElementPart",
            Class::IfcBuildingElementProxy => "IfcBuildingElementProxy",
            Class::IfcBuildingElementProxyType => "IfcBuildingElementProxyType",
            Class::IfcBuildingStorey => "IfcBuildingStorey",
            Class::IfcCShapeProfileDef => "IfcCShapeProfileDef",
            Class::IfcFlowFittingType => "IfcFlowFittingType",
            Class::IfcCableCarrierFittingType => "IfcCableCarrierFittingType",
            Class::IfcFlowSegmentType => "IfcFlowSegmentType",
            Class::IfcCableCarrierSegmentType => "IfcCableCarrierSegmentType",
            Class::IfcCableSegmentType => "IfcCableSegmentType",
            Class::IfcPoint => "IfcPoint",
            Class::IfcCartesianPoint => "IfcCartesianPoint",
            Class::IfcCartesianTransformationOperator => "IfcCartesianTransformationOperator",
            Class::IfcCartesianTransformationOperator2D => "IfcCartesianTransformationOperator2D",
            Class::IfcCartesianTransformationOperator2DnonUniform => "IfcCartesianTransformationOperator2DnonUniform",
            Class::IfcCartesianTransformationOperator3D => "IfcCartesianTransformationOperator3D",
            Class::IfcCartesianTransformationOperator3DnonUniform => "IfcCartesianTransformationOperator3DnonUniform",
            Class::IfcCenterLineProfileDef => "IfcCenterLineProfileDef",
            Class::IfcFeatureElement => "IfcFeatureElement",
            Class::IfcFeatureElementSubtraction => "IfcFeatureElementSubtraction",
            Class::IfcEdgeFeature => "IfcEdgeFeature",
            Class::IfcChamferEdgeFeature => "IfcChamferEdgeFeature",
            Class::IfcChillerType => "IfcChillerType",
            Class::IfcConic => "IfcConic",
            Class::IfcCircle => "IfcCircle",
            Class::IfcCircleProfileDef => "IfcCircleProfileDef",
            Class::IfcCircleHollowProfileDef => "IfcCircleHollowProfileDef",
            Class::IfcTopologicalRepresentationItem => "IfcTopologicalRepresentationItem",
            Class::IfcConnectedFaceSet => "IfcConnectedFaceSet",
            Class::IfcClosedShell => "IfcClosedShell",
            Class::IfcCoilType => "IfcCoilType",
            Class::IfcColourSpecification => "IfcColourSpecification",
            Class::IfcColourRgb => "IfcColourRgb",
            Class::IfcColumn => "IfcColumn",
            Class::IfcColumnType => "IfcColumnType",
            Class::IfcProperty => "IfcProperty",
            Class::IfcComplexProperty => "IfcComplexProperty",
            Class::IfcCompositeCurveSegment => "IfcCompositeCurveSegment",
            Class::IfcCompositeProfileDef => "IfcCompositeProfileDef",
            Class::IfcFlowMovingDeviceType => "IfcFlowMovingDeviceType",
            Class::IfcCompressorType => "IfcCompressorType",
            Class::IfcCondenserType => "IfcCondenserType",
            Class::IfcCondition => "IfcCondition",
            Class::IfcConditionCriterion => "IfcConditionCriterion",
            Class::IfcResource => "IfcResource",
            Class::IfcConstructionResource => "IfcConstructionResource",
            Class::IfcConstructionEquipmentResource => "IfcConstructionEquipmentResource",
            Class::IfcConstructionMaterialResource => "IfcConstructionMaterialResource",
            Class::IfcConstructionProductResource => "IfcConstructionProductResource",
            Class::IfcNamedUnit => "IfcNamedUnit",
            Class::IfcContextDependentUnit => "IfcContextDependentUnit",
            Class::IfcControllerType => "IfcControllerType",
            Class::IfcConversionBasedUnit => "IfcConversionBasedUnit",
            Class::IfcCooledBeamType => "IfcCooledBeamType",
            Class::IfcCoolingTowerType => "IfcCoolingTowerType",
            Class::IfcCostItem => "IfcCostItem",
            Class::IfcCostSchedule => "IfcCostSchedule",
            Class::IfcCovering => "IfcCovering",
            Class::IfcCoveringType => "IfcCoveringType",
            Class::IfcCraneRailAShapeProfileDef => "IfcCraneRailAShapeProfileDef",
            Class::IfcCraneRailFShapeProfileDef => "IfcCraneRailFShapeProfileDef",
            Class::IfcCrewResource => "IfcCrewResource",
            Class::IfcSolidModel => "IfcSolidModel",
            Class::IfcCsgSolid => "IfcCsgSolid",
            Class::IfcCurtainWall => "IfcCurtainWall",
            Class::IfcCurtainWallType => "IfcCurtainWallType",
            Class::IfcCurveBoundedPlane => "IfcCurveBoundedPlane",
            Class::IfcPresentationStyle => "IfcPresentationStyle",
            Class::IfcDamperType => "IfcDamperType",
            Class::IfcDefinedSymbol => "IfcDefinedSymbol",
            Class::IfcDerivedProfileDef => "IfcDerivedProfileDef",
            Class::IfcDiameterDimension => "IfcDiameterDimension",
            Class::IfcDimensionCurve => "IfcDimensionCurve",
            Class::IfcTerminatorSymbol => "IfcTerminatorSymbol",
            Class::IfcDimensionCurveTerminator => "IfcDimensionCurveTerminator",
            Class::IfcDirection => "IfcDirection",
            Class::IfcElementComponent => "IfcElementComponent",
            Class::IfcDiscreteAccessory => "IfcDiscreteAccessory",
            Class::IfcElementComponentType => "IfcElementComponentType",
            Class::IfcDiscreteAccessoryType => "IfcDiscreteAccessoryType",
            Class::IfcDistributionElement => "IfcDistributionElement",
            Class::IfcDistributionFlowElement => "IfcDistributionFlowElement",
            Class::IfcDistributionChamberElement => "IfcDistributionChamberElement",
            Class::IfcDistributionChamberElementType => "IfcDistributionChamberElementType",
            Class::IfcDistributionControlElement => "IfcDistributionControlElement",
            Class::IfcPort => "IfcPort",
            Class::IfcDistributionPort => "IfcDistributionPort",
            Class::IfcDoor => "IfcDoor",
            Class::IfcPropertyDefinition => "IfcPropertyDefinition",
            Class::IfcPropertySetDefinition => "IfcPropertySetDefinition",
            Class::IfcDoorStyle => "IfcDoorStyle",
            Class::IfcDuctFittingType => "IfcDuctFittingType",
            Class::IfcDuctSegmentType => "IfcDuctSegmentType",
            Class::IfcFlowTreatmentDeviceType => "IfcFlowTreatmentDeviceType",
            Class::IfcDuctSilencerType => "IfcDuctSilencerType",
            Class::IfcEdge => "IfcEdge",
            Class::IfcEdgeCurve => "IfcEdgeCurve",
            Class::IfcLoop => "IfcLoop",
            Class::IfcEdgeLoop => "IfcEdgeLoop",
            Class::IfcElectricApplianceType => "IfcElectricApplianceType",
            Class::IfcFlowController => "IfcFlowController",
            Class::IfcElectricDistributionPoint => "IfcElectricDistributionPoint",
            Class::IfcFlowStorageDeviceType => "IfcFlowStorageDeviceType",
            Class::IfcElectricFlowStorageDeviceType => "IfcElectricFlowStorageDeviceType",
            Class::IfcElectricGeneratorType => "IfcElectricGeneratorType",
            Class::IfcElectricHeaterType => "IfcElectricHeaterType",
            Class::IfcElectricMotorType => "IfcElectricMotorType",
            Class::IfcElectricTimeControlType => "IfcElectricTimeControlType",
            Class::IfcSystem => "IfcSystem",
            Class::IfcElectricalCircuit => "IfcElectricalCircuit",
            Class::IfcElectricalElement => "IfcElectricalElement",
            Class::IfcElementAssembly => "IfcElementAssembly",
            Class::IfcElementQuantity => "IfcElementQuantity",
            Class::IfcElementarySurface => "IfcElementarySurface",
            Class::IfcEllipse => "IfcEllipse",
            Class::IfcEllipseProfileDef => "IfcEllipseProfileDef",
            Class::IfcEnergyConversionDevice => "IfcEnergyConversionDevice",
            Class::IfcEquipmentElement => "IfcEquipmentElement",
            Class::IfcEquipmentStandard => "IfcEquipmentStandard",
            Class::IfcEvaporativeCoolerType => "IfcEvaporativeCoolerType",
            Class::IfcEvaporatorType => "IfcEvaporatorType",
            Class::IfcSweptAreaSolid => "IfcSweptAreaSolid",
            Class::IfcExtrudedAreaSolid => "IfcExtrudedAreaSolid",
            Class::IfcFace => "IfcFace",
            Class::IfcFaceBasedSurfaceModel => "IfcFaceBasedSurfaceModel",
            Class::IfcFaceBound => "IfcFaceBound",
            Class::IfcFaceOuterBound => "IfcFaceOuterBound",
            Class::IfcFaceSurface => "IfcFaceSurface",
            Class::IfcManifoldSolidBrep => "IfcManifoldSolidBrep",
            Class::IfcFacetedBrep => "IfcFacetedBrep",
            Class::IfcFacetedBrepWithVoids => "IfcFacetedBrepWithVoids",
            Class::IfcFanType => "IfcFanType",
            Class::IfcFastener => "IfcFastener",
            Class::IfcFastenerType => "IfcFastenerType",
            Class::IfcFeatureElementAddition => "IfcFeatureElementAddition",
            Class::IfcFillAreaStyleHatching => "IfcFillAreaStyleHatching",
            Class::IfcFillAreaStyleTileSymbolWithStyle => "IfcFillAreaStyleTileSymbolWithStyle",
            Class::IfcFillAreaStyleTiles => "IfcFillAreaStyleTiles",
            Class::IfcFilterType => "IfcFilterType",
            Class::IfcFireSuppressionTerminalType => "IfcFireSuppressionTerminalType",
            Class::IfcFlowFitting => "IfcFlowFitting",
            Class::IfcFlowInstrumentType => "IfcFlowInstrumentType",
            Class::IfcFlowMeterType => "IfcFlowMeterType",
            Class::IfcFlowMovingDevice => "IfcFlowMovingDevice",
            Class::IfcFlowSegment => "IfcFlowSegment",
            Class::IfcFlowStorageDevice => "IfcFlowStorageDevice",
            Class::IfcFlowTerminal => "IfcFlowTerminal",
            Class::IfcFlowTreatmentDevice => "IfcFlowTreatmentDevice",
            Class::IfcFooting => "IfcFooting",
            Class::IfcFurnishingElement => "IfcFurnishingElement",
            Class::IfcFurnishingElementType => "IfcFurnishingElementType",
            Class::IfcFurnitureStandard => "IfcFurnitureStandard",
            Class::IfcFurnitureType => "IfcFurnitureType",
            Class::IfcGasTerminalType => "IfcGasTerminalType",
            Class::IfcGeometricSet => "IfcGeometricSet",
            Class::IfcGeometricCurveSet => "IfcGeometricCurveSet",
            Class::IfcRepresentationContext => "IfcRepresentationContext",
            Class::IfcGeometricRepresentationContext => "IfcGeometricRepresentationContext",
            Class::IfcGeometricRepresentationSubContext => "IfcGeometricRepresentationSubContext",
            Class::IfcGrid => "IfcGrid",
            Class::IfcObjectPlacement => "IfcObjectPlacement",
            Class::IfcGridPlacement => "IfcGridPlacement",
            Class::IfcHeatExchangerType => "IfcHeatExchangerType",
            Class::IfcHumidifierType => "IfcHumidifierType",
            Class::IfcInventory => "IfcInventory",
            Class::IfcJunctionBoxType => "IfcJunctionBoxType",
            Class::IfcLShapeProfileDef => "IfcLShapeProfileDef",
            Class::IfcLaborResource => "IfcLaborResource",
            Class::IfcLampType => "IfcLampType",
            Class::IfcLightFixtureType => "IfcLightFixtureType",
            Class::IfcLightSource => "IfcLightSource",
            Class::IfcLightSourceAmbient => "IfcLightSourceAmbient",
            Class::IfcLightSourceDirectional => "IfcLightSourceDirectional",
            Class::IfcLightSourceGoniometric => "IfcLightSourceGoniometric",
            Class::IfcLightSourcePositional => "IfcLightSourcePositional",
            Class::IfcLightSourceSpot => "IfcLightSourceSpot",
            Class::IfcLine => "IfcLine",
            Class::IfcLinearDimension => "IfcLinearDimension",
            Class::IfcLocalPlacement => "IfcLocalPlacement",
            Class::IfcMappedItem => "IfcMappedItem",
            Class::IfcProductRepresentation => "IfcProductRepresentation",
            Class::IfcMaterialDefinitionRepresentation => "IfcMaterialDefinitionRepresentation",
            Class::IfcMeasureWithUnit => "IfcMeasureWithUnit",
            Class::IfcMechanicalFastener => "IfcMechanicalFastener",
            Class::IfcMechanicalFastenerType => "IfcMechanicalFastenerType",
            Class::IfcMember => "IfcMember",
            Class::IfcMemberType => "IfcMemberType",
            Class::IfcMotorConnectionType => "IfcMotorConnectionType",
            Class::IfcProcess => "IfcProcess",
            Class::IfcTask => "IfcTask",
            Class::IfcMove => "IfcMove",
            Class::IfcOccupant => "IfcOccupant",
            Class::IfcOffsetCurve2D => "IfcOffsetCurve2D",
            Class::IfcOffsetCurve3D => "IfcOffsetCurve3D",
            Class::IfcOneDirectionRepeatFactor => "IfcOneDirectionRepeatFactor",
            Class::IfcOpenShell => "IfcOpenShell",
            Class::IfcOpeningElement => "IfcOpeningElement",
            Class::IfcOrderAction => "IfcOrderAction",
            Class::IfcOrientedEdge => "IfcOrientedEdge",
            Class::IfcOutletType => "IfcOutletType",
            Class::IfcPath => "IfcPath",
            Class::IfcPerformanceHistory => "IfcPerformanceHistory",
            Class::IfcPermit => "IfcPermit",
            Class::IfcPile => "IfcPile",
            Class::IfcPipeFittingType => "IfcPipeFittingType",
            Class::IfcPipeSegmentType => "IfcPipeSegmentType",
            Class::IfcPlanarExtent => "IfcPlanarExtent",
            Class::IfcPlanarBox => "IfcPlanarBox",
            Class::IfcPlane => "IfcPlane",
            Class::IfcPlate => "IfcPlate",
            Class::IfcPlateType => "IfcPlateType",
            Class::IfcPointOnCurve => "IfcPointOnCurve",
            Class::IfcPointOnSurface => "IfcPointOnSurface",
            Class::IfcPolyLoop => "IfcPolyLoop",
            Class::IfcPolygonalBoundedHalfSpace => "IfcPolygonalBoundedHalfSpace",
            Class::IfcPolyline => "IfcPolyline",
            Class::IfcPresentationStyleAssignment => "IfcPresentationStyleAssignment",
            Class::IfcProcedure => "IfcProcedure",
            Class::IfcProductDefinitionShape => "IfcProductDefinitionShape",
            Class::IfcProject => "IfcProject",
            Class::IfcProjectOrder => "IfcProjectOrder",
            Class::IfcProjectOrderRecord => "IfcProjectOrderRecord",
            Class::IfcProjectionCurve => "IfcProjectionCurve",
            Class::IfcProjectionElement => "IfcProjectionElement",
            Class::IfcSimpleProperty => "IfcSimpleProperty",
            Class::IfcPropertyBoundedValue => "IfcPropertyBoundedValue",
            Class::IfcPropertyEnumeratedValue => "IfcPropertyEnumeratedValue",
            Class::IfcPropertyListValue => "IfcPropertyListValue",
            Class::IfcPropertyReferenceValue => "IfcPropertyReferenceValue",
            Class::IfcPropertySet => "IfcPropertySet",
            Class::IfcPropertySingleValue => "IfcPropertySingleValue",
            Class::IfcPropertyTableValue => "IfcPropertyTableValue",
            Class::IfcProtectiveDeviceType => "IfcProtectiveDeviceType",
            Class::IfcProxy => "IfcProxy",
            Class::IfcPumpType => "IfcPumpType",
            Class::IfcRadiusDimension => "IfcRadiusDimension",
            Class::IfcRailing => "IfcRailing",
            Class::IfcRailingType => "IfcRailingType",
            Class::IfcRamp => "IfcRamp",
            Class::IfcRampFlight => "IfcRampFlight",
            Class::IfcRampFlightType => "IfcRampFlightType",
            Class::IfcRationalBezierCurve => "IfcRationalBezierCurve",
            Class::IfcRectangleProfileDef => "IfcRectangleProfileDef",
            Class::IfcRectangleHollowProfileDef => "IfcRectangleHollowProfileDef",
            Class::IfcRectangularPyramid => "IfcRectangularPyramid",
            Class::IfcRectangularTrimmedSurface => "IfcRectangularTrimmedSurface",
            Class::IfcReinforcingElement => "IfcReinforcingElement",
            Class::IfcReinforcingBar => "IfcReinforcingBar",
            Class::IfcReinforcingMesh => "IfcReinforcingMesh",
            Class::IfcRelationship => "IfcRelationship",
            Class::IfcRelDecomposes => "IfcRelDecomposes",
            Class::IfcRelAggregates => "IfcRelAggregates",
            Class::IfcRelConnects => "IfcRelConnects",
            Class::IfcRelContainedInSpatialStructure => "IfcRelContainedInSpatialStructure",
            Class::IfcRelDefines => "IfcRelDefines",
            Class::IfcRelDefinesByProperties => "IfcRelDefinesByProperties",
            Class::IfcRelFillsElement => "IfcRelFillsElement",
            Class::IfcRelOverridesProperties => "IfcRelOverridesProperties",
            Class::IfcRelVoidsElement => "IfcRelVoidsElement",
            Class::IfcRepresentation => "IfcRepresentation",
            Class::IfcRepresentationMap => "IfcRepresentationMap",
            Class::IfcRevolvedAreaSolid => "IfcRevolvedAreaSolid",
            Class::IfcRightCircularCone => "IfcRightCircularCone",
            Class::IfcRightCircularCylinder => "IfcRightCircularCylinder",
            Class::IfcRoof => "IfcRoof",
            Class::IfcRoundedEdgeFeature => "IfcRoundedEdgeFeature",
            Class::IfcRoundedRectangleProfileDef => "IfcRoundedRectangleProfileDef",
            Class::IfcSIUnit => "IfcSIUnit",
            Class::IfcSanitaryTerminalType => "IfcSanitaryTerminalType",
            Class::IfcScheduleTimeControl => "IfcScheduleTimeControl",
            Class::IfcSectionedSpine => "IfcSectionedSpine",
            Class::IfcSensorType => "IfcSensorType",
            Class::IfcServiceLife => "IfcServiceLife",
            Class::IfcShapeModel => "IfcShapeModel",
            Class::IfcShapeRepresentation => "IfcShapeRepresentation",
            Class::IfcShellBasedSurfaceModel => "IfcShellBasedSurfaceModel",
            Class::IfcSite => "IfcSite",
            Class::IfcSlab => "IfcSlab",
            Class::IfcSlabType => "IfcSlabType",
            Class::IfcSpace => "IfcSpace",
            Class::IfcSpaceHeaterType => "IfcSpaceHeaterType",
            Class::IfcSpaceProgram => "IfcSpaceProgram",
            Class::IfcSpatialStructureElementType => "IfcSpatialStructureElementType",
            Class::IfcSpaceType => "IfcSpaceType",
            Class::IfcSphere => "IfcSphere",
            Class::IfcStackTerminalType => "IfcStackTerminalType",
            Class::IfcStair => "IfcStair",
            Class::IfcStairFlight => "IfcStairFlight",
            Class::IfcStairFlightType => "IfcStairFlightType",
            Class::IfcStructuralActivity => "IfcStructuralActivity",
            Class::IfcStructuralAction => "IfcStructuralAction",
            Class::IfcStructuralAnalysisModel => "IfcStructuralAnalysisModel",
            Class::IfcStructuralItem => "IfcStructuralItem",
            Class::IfcStructuralConnection => "IfcStructuralConnection",
            Class::IfcStructuralCurveConnection => "IfcStructuralCurveConnection",
            Class::IfcStructuralMember => "IfcStructuralMember",
            Class::IfcStructuralCurveMember => "IfcStructuralCurveMember",
            Class::IfcStructuralCurveMemberVarying => "IfcStructuralCurveMemberVarying",
            Class::IfcStructuralLinearAction => "IfcStructuralLinearAction",
            Class::IfcStructuralLinearActionVarying => "IfcStructuralLinearActionVarying",
            Class::IfcStructuralLoadGroup => "IfcStructuralLoadGroup",
            Class::IfcStructuralPlanarAction => "IfcStructuralPlanarAction",
            Class::IfcStructuralPlanarActionVarying => "IfcStructuralPlanarActionVarying",
            Class::IfcStructuralPointAction => "IfcStructuralPointAction",
            Class::IfcStructuralPointConnection => "IfcStructuralPointConnection",
            Class::IfcStructuralReaction => "IfcStructuralReaction",
            Class::IfcStructuralPointReaction => "IfcStructuralPointReaction",
            Class::IfcStructuralResultGroup => "IfcStructuralResultGroup",
            Class::IfcStructuralSurfaceConnection => "IfcStructuralSurfaceConnection",
            Class::IfcStructuralSurfaceMember => "IfcStructuralSurfaceMember",
            Class::IfcStructuralSurfaceMemberVarying => "IfcStructuralSurfaceMemberVarying",
            Class::IfcStructuredDimensionCallout => "IfcStructuredDimensionCallout",
            Class::IfcStyleModel => "IfcStyleModel",
            Class::IfcStyledRepresentation => "IfcStyledRepresentation",
            Class::IfcSubContractResource => "IfcSubContractResource",
            Class::IfcSubedge => "IfcSubedge",
            Class::IfcSurfaceCurveSweptAreaSolid => "IfcSurfaceCurveSweptAreaSolid",
            Class::IfcSweptSurface => "IfcSweptSurface",
            Class::IfcSurfaceOfLinearExtrusion => "IfcSurfaceOfLinearExtrusion",
            Class::IfcSurfaceOfRevolution => "IfcSurfaceOfRevolution",
            Class::IfcSurfaceStyle => "IfcSurfaceStyle",
            Class::IfcSurfaceStyleShading => "IfcSurfaceStyleShading",
            Class::IfcSurfaceStyleRendering => "IfcSurfaceStyleRendering",
            Class::IfcSurfaceStyleWithTextures => "IfcSurfaceStyleWithTextures",
            Class::IfcSweptDiskSolid => "IfcSweptDiskSolid",
            Class::IfcSwitchingDeviceType => "IfcSwitchingDeviceType",
            Class::IfcSystemFurnitureElementType => "IfcSystemFurnitureElementType",
            Class::IfcTShapeProfileDef => "IfcTShapeProfileDef",
            Class::IfcTankType => "IfcTankType",
            Class::IfcTendon => "IfcTendon",
            Class::IfcTendonAnchor => "IfcTendonAnchor",
            Class::IfcTextLiteral => "IfcTextLiteral",
            Class::IfcTextLiteralWithExtent => "IfcTextLiteralWithExtent",
            Class::IfcTimeSeriesSchedule => "IfcTimeSeriesSchedule",
            Class::IfcTopologyRepresentation => "IfcTopologyRepresentation",
            Class::IfcTransformerType => "IfcTransformerType",
            Class::IfcTransportElement => "IfcTransportElement",
            Class::IfcTransportElementType => "IfcTransportElementType",
            Class::IfcTrapeziumProfileDef => "IfcTrapeziumProfileDef",
            Class::IfcTrimmedCurve => "IfcTrimmedCurve",
            Class::IfcTubeBundleType => "IfcTubeBundleType",
            Class::IfcTwoDirectionRepeatFactor => "IfcTwoDirectionRepeatFactor",
            Class::IfcUShapeProfileDef => "IfcUShapeProfileDef",
            Class::IfcUnitAssignment => "IfcUnitAssignment",
            Class::IfcUnitaryEquipmentType => "IfcUnitaryEquipmentType",
            Class::IfcValveType => "IfcValveType",
            Class::IfcVector => "IfcVector",
            Class::IfcVertex => "IfcVertex",
            Class::IfcVertexLoop => "IfcVertexLoop",
            Class::IfcVertexPoint => "IfcVertexPoint",
            Class::IfcVibrationIsolatorType => "IfcVibrationIsolatorType",
            Class::IfcVirtualElement => "IfcVirtualElement",
            Class::IfcWall => "IfcWall",
            Class::IfcWallStandardCase => "IfcWallStandardCase",
            Class::IfcWallType => "IfcWallType",
            Class::IfcWasteTerminalType => "IfcWasteTerminalType",
            Class::IfcWindow => "IfcWindow",
            Class::IfcWindowStyle => "IfcWindowStyle",
            Class::IfcWorkControl => "IfcWorkControl",
            Class::IfcWorkPlan => "IfcWorkPlan",
            Class::IfcWorkSchedule => "IfcWorkSchedule",
            Class::IfcZShapeProfileDef => "IfcZShapeProfileDef",
            Class::IfcZone => "IfcZone",
        }
    }

    /// Whether an object of this class is a `class` (`dynamic_cast`).
    pub fn is_a(self, class: Class) -> bool {
        let mut current = Some(self);
        while let Some(candidate) = current {
            if candidate == class {
                return true;
            }
            current = candidate.parent();
        }
        false
    }

    /// The class's own `GenericFill` step.
    pub fn fill(self) -> &'static ClassFill {
        match self {
            Class::IfcRepresentationItem => &IFC_REPRESENTATION_ITEM_FILL,
            Class::IfcGeometricRepresentationItem => &IFC_GEOMETRIC_REPRESENTATION_ITEM_FILL,
            Class::IfcCurve => &IFC_CURVE_FILL,
            Class::IfcBoundedCurve => &IFC_BOUNDED_CURVE_FILL,
            Class::IfcCompositeCurve => &IFC_COMPOSITE_CURVE_FILL,
            Class::Ifc2DCompositeCurve => &IFC2_D_COMPOSITE_CURVE_FILL,
            Class::IfcRoot => &IFC_ROOT_FILL,
            Class::IfcObjectDefinition => &IFC_OBJECT_DEFINITION_FILL,
            Class::IfcObject => &IFC_OBJECT_FILL,
            Class::IfcControl => &IFC_CONTROL_FILL,
            Class::IfcActionRequest => &IFC_ACTION_REQUEST_FILL,
            Class::IfcActor => &IFC_ACTOR_FILL,
            Class::IfcTypeObject => &IFC_TYPE_OBJECT_FILL,
            Class::IfcTypeProduct => &IFC_TYPE_PRODUCT_FILL,
            Class::IfcElementType => &IFC_ELEMENT_TYPE_FILL,
            Class::IfcDistributionElementType => &IFC_DISTRIBUTION_ELEMENT_TYPE_FILL,
            Class::IfcDistributionControlElementType => &IFC_DISTRIBUTION_CONTROL_ELEMENT_TYPE_FILL,
            Class::IfcActuatorType => &IFC_ACTUATOR_TYPE_FILL,
            Class::IfcDistributionFlowElementType => &IFC_DISTRIBUTION_FLOW_ELEMENT_TYPE_FILL,
            Class::IfcFlowControllerType => &IFC_FLOW_CONTROLLER_TYPE_FILL,
            Class::IfcAirTerminalBoxType => &IFC_AIR_TERMINAL_BOX_TYPE_FILL,
            Class::IfcFlowTerminalType => &IFC_FLOW_TERMINAL_TYPE_FILL,
            Class::IfcAirTerminalType => &IFC_AIR_TERMINAL_TYPE_FILL,
            Class::IfcEnergyConversionDeviceType => &IFC_ENERGY_CONVERSION_DEVICE_TYPE_FILL,
            Class::IfcAirToAirHeatRecoveryType => &IFC_AIR_TO_AIR_HEAT_RECOVERY_TYPE_FILL,
            Class::IfcAlarmType => &IFC_ALARM_TYPE_FILL,
            Class::IfcDraughtingCallout => &IFC_DRAUGHTING_CALLOUT_FILL,
            Class::IfcDimensionCurveDirectedCallout => &IFC_DIMENSION_CURVE_DIRECTED_CALLOUT_FILL,
            Class::IfcAngularDimension => &IFC_ANGULAR_DIMENSION_FILL,
            Class::IfcProduct => &IFC_PRODUCT_FILL,
            Class::IfcAnnotation => &IFC_ANNOTATION_FILL,
            Class::IfcStyledItem => &IFC_STYLED_ITEM_FILL,
            Class::IfcAnnotationOccurrence => &IFC_ANNOTATION_OCCURRENCE_FILL,
            Class::IfcAnnotationCurveOccurrence => &IFC_ANNOTATION_CURVE_OCCURRENCE_FILL,
            Class::IfcAnnotationFillArea => &IFC_ANNOTATION_FILL_AREA_FILL,
            Class::IfcAnnotationFillAreaOccurrence => &IFC_ANNOTATION_FILL_AREA_OCCURRENCE_FILL,
            Class::IfcAnnotationSurface => &IFC_ANNOTATION_SURFACE_FILL,
            Class::IfcAnnotationSurfaceOccurrence => &IFC_ANNOTATION_SURFACE_OCCURRENCE_FILL,
            Class::IfcAnnotationSymbolOccurrence => &IFC_ANNOTATION_SYMBOL_OCCURRENCE_FILL,
            Class::IfcAnnotationTextOccurrence => &IFC_ANNOTATION_TEXT_OCCURRENCE_FILL,
            Class::IfcProfileDef => &IFC_PROFILE_DEF_FILL,
            Class::IfcArbitraryClosedProfileDef => &IFC_ARBITRARY_CLOSED_PROFILE_DEF_FILL,
            Class::IfcArbitraryOpenProfileDef => &IFC_ARBITRARY_OPEN_PROFILE_DEF_FILL,
            Class::IfcArbitraryProfileDefWithVoids => &IFC_ARBITRARY_PROFILE_DEF_WITH_VOIDS_FILL,
            Class::IfcGroup => &IFC_GROUP_FILL,
            Class::IfcAsset => &IFC_ASSET_FILL,
            Class::IfcParameterizedProfileDef => &IFC_PARAMETERIZED_PROFILE_DEF_FILL,
            Class::IfcIShapeProfileDef => &IFC_I_SHAPE_PROFILE_DEF_FILL,
            Class::IfcAsymmetricIShapeProfileDef => &IFC_ASYMMETRIC_I_SHAPE_PROFILE_DEF_FILL,
            Class::IfcPlacement => &IFC_PLACEMENT_FILL,
            Class::IfcAxis1Placement => &IFC_AXIS1_PLACEMENT_FILL,
            Class::IfcAxis2Placement2D => &IFC_AXIS2_PLACEMENT2_D_FILL,
            Class::IfcAxis2Placement3D => &IFC_AXIS2_PLACEMENT3_D_FILL,
            Class::IfcBSplineCurve => &IFC_B_SPLINE_CURVE_FILL,
            Class::IfcElement => &IFC_ELEMENT_FILL,
            Class::IfcBuildingElement => &IFC_BUILDING_ELEMENT_FILL,
            Class::IfcBeam => &IFC_BEAM_FILL,
            Class::IfcBuildingElementType => &IFC_BUILDING_ELEMENT_TYPE_FILL,
            Class::IfcBeamType => &IFC_BEAM_TYPE_FILL,
            Class::IfcBezierCurve => &IFC_BEZIER_CURVE_FILL,
            Class::IfcCsgPrimitive3D => &IFC_CSG_PRIMITIVE3_D_FILL,
            Class::IfcBlock => &IFC_BLOCK_FILL,
            Class::IfcBoilerType => &IFC_BOILER_TYPE_FILL,
            Class::IfcBooleanResult => &IFC_BOOLEAN_RESULT_FILL,
            Class::IfcBooleanClippingResult => &IFC_BOOLEAN_CLIPPING_RESULT_FILL,
            Class::IfcSurface => &IFC_SURFACE_FILL,
            Class::IfcBoundedSurface => &IFC_BOUNDED_SURFACE_FILL,
            Class::IfcBoundingBox => &IFC_BOUNDING_BOX_FILL,
            Class::IfcHalfSpaceSolid => &IFC_HALF_SPACE_SOLID_FILL,
            Class::IfcBoxedHalfSpace => &IFC_BOXED_HALF_SPACE_FILL,
            Class::IfcSpatialStructureElement => &IFC_SPATIAL_STRUCTURE_ELEMENT_FILL,
            Class::IfcBuilding => &IFC_BUILDING_FILL,
            Class::IfcBuildingElementComponent => &IFC_BUILDING_ELEMENT_COMPONENT_FILL,
            Class::IfcBuildingElementPart => &IFC_BUILDING_ELEMENT_PART_FILL,
            Class::IfcBuildingElementProxy => &IFC_BUILDING_ELEMENT_PROXY_FILL,
            Class::IfcBuildingElementProxyType => &IFC_BUILDING_ELEMENT_PROXY_TYPE_FILL,
            Class::IfcBuildingStorey => &IFC_BUILDING_STOREY_FILL,
            Class::IfcCShapeProfileDef => &IFC_C_SHAPE_PROFILE_DEF_FILL,
            Class::IfcFlowFittingType => &IFC_FLOW_FITTING_TYPE_FILL,
            Class::IfcCableCarrierFittingType => &IFC_CABLE_CARRIER_FITTING_TYPE_FILL,
            Class::IfcFlowSegmentType => &IFC_FLOW_SEGMENT_TYPE_FILL,
            Class::IfcCableCarrierSegmentType => &IFC_CABLE_CARRIER_SEGMENT_TYPE_FILL,
            Class::IfcCableSegmentType => &IFC_CABLE_SEGMENT_TYPE_FILL,
            Class::IfcPoint => &IFC_POINT_FILL,
            Class::IfcCartesianPoint => &IFC_CARTESIAN_POINT_FILL,
            Class::IfcCartesianTransformationOperator => &IFC_CARTESIAN_TRANSFORMATION_OPERATOR_FILL,
            Class::IfcCartesianTransformationOperator2D => &IFC_CARTESIAN_TRANSFORMATION_OPERATOR2_D_FILL,
            Class::IfcCartesianTransformationOperator2DnonUniform => &IFC_CARTESIAN_TRANSFORMATION_OPERATOR2_DNON_UNIFORM_FILL,
            Class::IfcCartesianTransformationOperator3D => &IFC_CARTESIAN_TRANSFORMATION_OPERATOR3_D_FILL,
            Class::IfcCartesianTransformationOperator3DnonUniform => &IFC_CARTESIAN_TRANSFORMATION_OPERATOR3_DNON_UNIFORM_FILL,
            Class::IfcCenterLineProfileDef => &IFC_CENTER_LINE_PROFILE_DEF_FILL,
            Class::IfcFeatureElement => &IFC_FEATURE_ELEMENT_FILL,
            Class::IfcFeatureElementSubtraction => &IFC_FEATURE_ELEMENT_SUBTRACTION_FILL,
            Class::IfcEdgeFeature => &IFC_EDGE_FEATURE_FILL,
            Class::IfcChamferEdgeFeature => &IFC_CHAMFER_EDGE_FEATURE_FILL,
            Class::IfcChillerType => &IFC_CHILLER_TYPE_FILL,
            Class::IfcConic => &IFC_CONIC_FILL,
            Class::IfcCircle => &IFC_CIRCLE_FILL,
            Class::IfcCircleProfileDef => &IFC_CIRCLE_PROFILE_DEF_FILL,
            Class::IfcCircleHollowProfileDef => &IFC_CIRCLE_HOLLOW_PROFILE_DEF_FILL,
            Class::IfcTopologicalRepresentationItem => &IFC_TOPOLOGICAL_REPRESENTATION_ITEM_FILL,
            Class::IfcConnectedFaceSet => &IFC_CONNECTED_FACE_SET_FILL,
            Class::IfcClosedShell => &IFC_CLOSED_SHELL_FILL,
            Class::IfcCoilType => &IFC_COIL_TYPE_FILL,
            Class::IfcColourSpecification => &IFC_COLOUR_SPECIFICATION_FILL,
            Class::IfcColourRgb => &IFC_COLOUR_RGB_FILL,
            Class::IfcColumn => &IFC_COLUMN_FILL,
            Class::IfcColumnType => &IFC_COLUMN_TYPE_FILL,
            Class::IfcProperty => &IFC_PROPERTY_FILL,
            Class::IfcComplexProperty => &IFC_COMPLEX_PROPERTY_FILL,
            Class::IfcCompositeCurveSegment => &IFC_COMPOSITE_CURVE_SEGMENT_FILL,
            Class::IfcCompositeProfileDef => &IFC_COMPOSITE_PROFILE_DEF_FILL,
            Class::IfcFlowMovingDeviceType => &IFC_FLOW_MOVING_DEVICE_TYPE_FILL,
            Class::IfcCompressorType => &IFC_COMPRESSOR_TYPE_FILL,
            Class::IfcCondenserType => &IFC_CONDENSER_TYPE_FILL,
            Class::IfcCondition => &IFC_CONDITION_FILL,
            Class::IfcConditionCriterion => &IFC_CONDITION_CRITERION_FILL,
            Class::IfcResource => &IFC_RESOURCE_FILL,
            Class::IfcConstructionResource => &IFC_CONSTRUCTION_RESOURCE_FILL,
            Class::IfcConstructionEquipmentResource => &IFC_CONSTRUCTION_EQUIPMENT_RESOURCE_FILL,
            Class::IfcConstructionMaterialResource => &IFC_CONSTRUCTION_MATERIAL_RESOURCE_FILL,
            Class::IfcConstructionProductResource => &IFC_CONSTRUCTION_PRODUCT_RESOURCE_FILL,
            Class::IfcNamedUnit => &IFC_NAMED_UNIT_FILL,
            Class::IfcContextDependentUnit => &IFC_CONTEXT_DEPENDENT_UNIT_FILL,
            Class::IfcControllerType => &IFC_CONTROLLER_TYPE_FILL,
            Class::IfcConversionBasedUnit => &IFC_CONVERSION_BASED_UNIT_FILL,
            Class::IfcCooledBeamType => &IFC_COOLED_BEAM_TYPE_FILL,
            Class::IfcCoolingTowerType => &IFC_COOLING_TOWER_TYPE_FILL,
            Class::IfcCostItem => &IFC_COST_ITEM_FILL,
            Class::IfcCostSchedule => &IFC_COST_SCHEDULE_FILL,
            Class::IfcCovering => &IFC_COVERING_FILL,
            Class::IfcCoveringType => &IFC_COVERING_TYPE_FILL,
            Class::IfcCraneRailAShapeProfileDef => &IFC_CRANE_RAIL_A_SHAPE_PROFILE_DEF_FILL,
            Class::IfcCraneRailFShapeProfileDef => &IFC_CRANE_RAIL_F_SHAPE_PROFILE_DEF_FILL,
            Class::IfcCrewResource => &IFC_CREW_RESOURCE_FILL,
            Class::IfcSolidModel => &IFC_SOLID_MODEL_FILL,
            Class::IfcCsgSolid => &IFC_CSG_SOLID_FILL,
            Class::IfcCurtainWall => &IFC_CURTAIN_WALL_FILL,
            Class::IfcCurtainWallType => &IFC_CURTAIN_WALL_TYPE_FILL,
            Class::IfcCurveBoundedPlane => &IFC_CURVE_BOUNDED_PLANE_FILL,
            Class::IfcPresentationStyle => &IFC_PRESENTATION_STYLE_FILL,
            Class::IfcDamperType => &IFC_DAMPER_TYPE_FILL,
            Class::IfcDefinedSymbol => &IFC_DEFINED_SYMBOL_FILL,
            Class::IfcDerivedProfileDef => &IFC_DERIVED_PROFILE_DEF_FILL,
            Class::IfcDiameterDimension => &IFC_DIAMETER_DIMENSION_FILL,
            Class::IfcDimensionCurve => &IFC_DIMENSION_CURVE_FILL,
            Class::IfcTerminatorSymbol => &IFC_TERMINATOR_SYMBOL_FILL,
            Class::IfcDimensionCurveTerminator => &IFC_DIMENSION_CURVE_TERMINATOR_FILL,
            Class::IfcDirection => &IFC_DIRECTION_FILL,
            Class::IfcElementComponent => &IFC_ELEMENT_COMPONENT_FILL,
            Class::IfcDiscreteAccessory => &IFC_DISCRETE_ACCESSORY_FILL,
            Class::IfcElementComponentType => &IFC_ELEMENT_COMPONENT_TYPE_FILL,
            Class::IfcDiscreteAccessoryType => &IFC_DISCRETE_ACCESSORY_TYPE_FILL,
            Class::IfcDistributionElement => &IFC_DISTRIBUTION_ELEMENT_FILL,
            Class::IfcDistributionFlowElement => &IFC_DISTRIBUTION_FLOW_ELEMENT_FILL,
            Class::IfcDistributionChamberElement => &IFC_DISTRIBUTION_CHAMBER_ELEMENT_FILL,
            Class::IfcDistributionChamberElementType => &IFC_DISTRIBUTION_CHAMBER_ELEMENT_TYPE_FILL,
            Class::IfcDistributionControlElement => &IFC_DISTRIBUTION_CONTROL_ELEMENT_FILL,
            Class::IfcPort => &IFC_PORT_FILL,
            Class::IfcDistributionPort => &IFC_DISTRIBUTION_PORT_FILL,
            Class::IfcDoor => &IFC_DOOR_FILL,
            Class::IfcPropertyDefinition => &IFC_PROPERTY_DEFINITION_FILL,
            Class::IfcPropertySetDefinition => &IFC_PROPERTY_SET_DEFINITION_FILL,
            Class::IfcDoorStyle => &IFC_DOOR_STYLE_FILL,
            Class::IfcDuctFittingType => &IFC_DUCT_FITTING_TYPE_FILL,
            Class::IfcDuctSegmentType => &IFC_DUCT_SEGMENT_TYPE_FILL,
            Class::IfcFlowTreatmentDeviceType => &IFC_FLOW_TREATMENT_DEVICE_TYPE_FILL,
            Class::IfcDuctSilencerType => &IFC_DUCT_SILENCER_TYPE_FILL,
            Class::IfcEdge => &IFC_EDGE_FILL,
            Class::IfcEdgeCurve => &IFC_EDGE_CURVE_FILL,
            Class::IfcLoop => &IFC_LOOP_FILL,
            Class::IfcEdgeLoop => &IFC_EDGE_LOOP_FILL,
            Class::IfcElectricApplianceType => &IFC_ELECTRIC_APPLIANCE_TYPE_FILL,
            Class::IfcFlowController => &IFC_FLOW_CONTROLLER_FILL,
            Class::IfcElectricDistributionPoint => &IFC_ELECTRIC_DISTRIBUTION_POINT_FILL,
            Class::IfcFlowStorageDeviceType => &IFC_FLOW_STORAGE_DEVICE_TYPE_FILL,
            Class::IfcElectricFlowStorageDeviceType => &IFC_ELECTRIC_FLOW_STORAGE_DEVICE_TYPE_FILL,
            Class::IfcElectricGeneratorType => &IFC_ELECTRIC_GENERATOR_TYPE_FILL,
            Class::IfcElectricHeaterType => &IFC_ELECTRIC_HEATER_TYPE_FILL,
            Class::IfcElectricMotorType => &IFC_ELECTRIC_MOTOR_TYPE_FILL,
            Class::IfcElectricTimeControlType => &IFC_ELECTRIC_TIME_CONTROL_TYPE_FILL,
            Class::IfcSystem => &IFC_SYSTEM_FILL,
            Class::IfcElectricalCircuit => &IFC_ELECTRICAL_CIRCUIT_FILL,
            Class::IfcElectricalElement => &IFC_ELECTRICAL_ELEMENT_FILL,
            Class::IfcElementAssembly => &IFC_ELEMENT_ASSEMBLY_FILL,
            Class::IfcElementQuantity => &IFC_ELEMENT_QUANTITY_FILL,
            Class::IfcElementarySurface => &IFC_ELEMENTARY_SURFACE_FILL,
            Class::IfcEllipse => &IFC_ELLIPSE_FILL,
            Class::IfcEllipseProfileDef => &IFC_ELLIPSE_PROFILE_DEF_FILL,
            Class::IfcEnergyConversionDevice => &IFC_ENERGY_CONVERSION_DEVICE_FILL,
            Class::IfcEquipmentElement => &IFC_EQUIPMENT_ELEMENT_FILL,
            Class::IfcEquipmentStandard => &IFC_EQUIPMENT_STANDARD_FILL,
            Class::IfcEvaporativeCoolerType => &IFC_EVAPORATIVE_COOLER_TYPE_FILL,
            Class::IfcEvaporatorType => &IFC_EVAPORATOR_TYPE_FILL,
            Class::IfcSweptAreaSolid => &IFC_SWEPT_AREA_SOLID_FILL,
            Class::IfcExtrudedAreaSolid => &IFC_EXTRUDED_AREA_SOLID_FILL,
            Class::IfcFace => &IFC_FACE_FILL,
            Class::IfcFaceBasedSurfaceModel => &IFC_FACE_BASED_SURFACE_MODEL_FILL,
            Class::IfcFaceBound => &IFC_FACE_BOUND_FILL,
            Class::IfcFaceOuterBound => &IFC_FACE_OUTER_BOUND_FILL,
            Class::IfcFaceSurface => &IFC_FACE_SURFACE_FILL,
            Class::IfcManifoldSolidBrep => &IFC_MANIFOLD_SOLID_BREP_FILL,
            Class::IfcFacetedBrep => &IFC_FACETED_BREP_FILL,
            Class::IfcFacetedBrepWithVoids => &IFC_FACETED_BREP_WITH_VOIDS_FILL,
            Class::IfcFanType => &IFC_FAN_TYPE_FILL,
            Class::IfcFastener => &IFC_FASTENER_FILL,
            Class::IfcFastenerType => &IFC_FASTENER_TYPE_FILL,
            Class::IfcFeatureElementAddition => &IFC_FEATURE_ELEMENT_ADDITION_FILL,
            Class::IfcFillAreaStyleHatching => &IFC_FILL_AREA_STYLE_HATCHING_FILL,
            Class::IfcFillAreaStyleTileSymbolWithStyle => &IFC_FILL_AREA_STYLE_TILE_SYMBOL_WITH_STYLE_FILL,
            Class::IfcFillAreaStyleTiles => &IFC_FILL_AREA_STYLE_TILES_FILL,
            Class::IfcFilterType => &IFC_FILTER_TYPE_FILL,
            Class::IfcFireSuppressionTerminalType => &IFC_FIRE_SUPPRESSION_TERMINAL_TYPE_FILL,
            Class::IfcFlowFitting => &IFC_FLOW_FITTING_FILL,
            Class::IfcFlowInstrumentType => &IFC_FLOW_INSTRUMENT_TYPE_FILL,
            Class::IfcFlowMeterType => &IFC_FLOW_METER_TYPE_FILL,
            Class::IfcFlowMovingDevice => &IFC_FLOW_MOVING_DEVICE_FILL,
            Class::IfcFlowSegment => &IFC_FLOW_SEGMENT_FILL,
            Class::IfcFlowStorageDevice => &IFC_FLOW_STORAGE_DEVICE_FILL,
            Class::IfcFlowTerminal => &IFC_FLOW_TERMINAL_FILL,
            Class::IfcFlowTreatmentDevice => &IFC_FLOW_TREATMENT_DEVICE_FILL,
            Class::IfcFooting => &IFC_FOOTING_FILL,
            Class::IfcFurnishingElement => &IFC_FURNISHING_ELEMENT_FILL,
            Class::IfcFurnishingElementType => &IFC_FURNISHING_ELEMENT_TYPE_FILL,
            Class::IfcFurnitureStandard => &IFC_FURNITURE_STANDARD_FILL,
            Class::IfcFurnitureType => &IFC_FURNITURE_TYPE_FILL,
            Class::IfcGasTerminalType => &IFC_GAS_TERMINAL_TYPE_FILL,
            Class::IfcGeometricSet => &IFC_GEOMETRIC_SET_FILL,
            Class::IfcGeometricCurveSet => &IFC_GEOMETRIC_CURVE_SET_FILL,
            Class::IfcRepresentationContext => &IFC_REPRESENTATION_CONTEXT_FILL,
            Class::IfcGeometricRepresentationContext => &IFC_GEOMETRIC_REPRESENTATION_CONTEXT_FILL,
            Class::IfcGeometricRepresentationSubContext => &IFC_GEOMETRIC_REPRESENTATION_SUB_CONTEXT_FILL,
            Class::IfcGrid => &IFC_GRID_FILL,
            Class::IfcObjectPlacement => &IFC_OBJECT_PLACEMENT_FILL,
            Class::IfcGridPlacement => &IFC_GRID_PLACEMENT_FILL,
            Class::IfcHeatExchangerType => &IFC_HEAT_EXCHANGER_TYPE_FILL,
            Class::IfcHumidifierType => &IFC_HUMIDIFIER_TYPE_FILL,
            Class::IfcInventory => &IFC_INVENTORY_FILL,
            Class::IfcJunctionBoxType => &IFC_JUNCTION_BOX_TYPE_FILL,
            Class::IfcLShapeProfileDef => &IFC_L_SHAPE_PROFILE_DEF_FILL,
            Class::IfcLaborResource => &IFC_LABOR_RESOURCE_FILL,
            Class::IfcLampType => &IFC_LAMP_TYPE_FILL,
            Class::IfcLightFixtureType => &IFC_LIGHT_FIXTURE_TYPE_FILL,
            Class::IfcLightSource => &IFC_LIGHT_SOURCE_FILL,
            Class::IfcLightSourceAmbient => &IFC_LIGHT_SOURCE_AMBIENT_FILL,
            Class::IfcLightSourceDirectional => &IFC_LIGHT_SOURCE_DIRECTIONAL_FILL,
            Class::IfcLightSourceGoniometric => &IFC_LIGHT_SOURCE_GONIOMETRIC_FILL,
            Class::IfcLightSourcePositional => &IFC_LIGHT_SOURCE_POSITIONAL_FILL,
            Class::IfcLightSourceSpot => &IFC_LIGHT_SOURCE_SPOT_FILL,
            Class::IfcLine => &IFC_LINE_FILL,
            Class::IfcLinearDimension => &IFC_LINEAR_DIMENSION_FILL,
            Class::IfcLocalPlacement => &IFC_LOCAL_PLACEMENT_FILL,
            Class::IfcMappedItem => &IFC_MAPPED_ITEM_FILL,
            Class::IfcProductRepresentation => &IFC_PRODUCT_REPRESENTATION_FILL,
            Class::IfcMaterialDefinitionRepresentation => &IFC_MATERIAL_DEFINITION_REPRESENTATION_FILL,
            Class::IfcMeasureWithUnit => &IFC_MEASURE_WITH_UNIT_FILL,
            Class::IfcMechanicalFastener => &IFC_MECHANICAL_FASTENER_FILL,
            Class::IfcMechanicalFastenerType => &IFC_MECHANICAL_FASTENER_TYPE_FILL,
            Class::IfcMember => &IFC_MEMBER_FILL,
            Class::IfcMemberType => &IFC_MEMBER_TYPE_FILL,
            Class::IfcMotorConnectionType => &IFC_MOTOR_CONNECTION_TYPE_FILL,
            Class::IfcProcess => &IFC_PROCESS_FILL,
            Class::IfcTask => &IFC_TASK_FILL,
            Class::IfcMove => &IFC_MOVE_FILL,
            Class::IfcOccupant => &IFC_OCCUPANT_FILL,
            Class::IfcOffsetCurve2D => &IFC_OFFSET_CURVE2_D_FILL,
            Class::IfcOffsetCurve3D => &IFC_OFFSET_CURVE3_D_FILL,
            Class::IfcOneDirectionRepeatFactor => &IFC_ONE_DIRECTION_REPEAT_FACTOR_FILL,
            Class::IfcOpenShell => &IFC_OPEN_SHELL_FILL,
            Class::IfcOpeningElement => &IFC_OPENING_ELEMENT_FILL,
            Class::IfcOrderAction => &IFC_ORDER_ACTION_FILL,
            Class::IfcOrientedEdge => &IFC_ORIENTED_EDGE_FILL,
            Class::IfcOutletType => &IFC_OUTLET_TYPE_FILL,
            Class::IfcPath => &IFC_PATH_FILL,
            Class::IfcPerformanceHistory => &IFC_PERFORMANCE_HISTORY_FILL,
            Class::IfcPermit => &IFC_PERMIT_FILL,
            Class::IfcPile => &IFC_PILE_FILL,
            Class::IfcPipeFittingType => &IFC_PIPE_FITTING_TYPE_FILL,
            Class::IfcPipeSegmentType => &IFC_PIPE_SEGMENT_TYPE_FILL,
            Class::IfcPlanarExtent => &IFC_PLANAR_EXTENT_FILL,
            Class::IfcPlanarBox => &IFC_PLANAR_BOX_FILL,
            Class::IfcPlane => &IFC_PLANE_FILL,
            Class::IfcPlate => &IFC_PLATE_FILL,
            Class::IfcPlateType => &IFC_PLATE_TYPE_FILL,
            Class::IfcPointOnCurve => &IFC_POINT_ON_CURVE_FILL,
            Class::IfcPointOnSurface => &IFC_POINT_ON_SURFACE_FILL,
            Class::IfcPolyLoop => &IFC_POLY_LOOP_FILL,
            Class::IfcPolygonalBoundedHalfSpace => &IFC_POLYGONAL_BOUNDED_HALF_SPACE_FILL,
            Class::IfcPolyline => &IFC_POLYLINE_FILL,
            Class::IfcPresentationStyleAssignment => &IFC_PRESENTATION_STYLE_ASSIGNMENT_FILL,
            Class::IfcProcedure => &IFC_PROCEDURE_FILL,
            Class::IfcProductDefinitionShape => &IFC_PRODUCT_DEFINITION_SHAPE_FILL,
            Class::IfcProject => &IFC_PROJECT_FILL,
            Class::IfcProjectOrder => &IFC_PROJECT_ORDER_FILL,
            Class::IfcProjectOrderRecord => &IFC_PROJECT_ORDER_RECORD_FILL,
            Class::IfcProjectionCurve => &IFC_PROJECTION_CURVE_FILL,
            Class::IfcProjectionElement => &IFC_PROJECTION_ELEMENT_FILL,
            Class::IfcSimpleProperty => &IFC_SIMPLE_PROPERTY_FILL,
            Class::IfcPropertyBoundedValue => &IFC_PROPERTY_BOUNDED_VALUE_FILL,
            Class::IfcPropertyEnumeratedValue => &IFC_PROPERTY_ENUMERATED_VALUE_FILL,
            Class::IfcPropertyListValue => &IFC_PROPERTY_LIST_VALUE_FILL,
            Class::IfcPropertyReferenceValue => &IFC_PROPERTY_REFERENCE_VALUE_FILL,
            Class::IfcPropertySet => &IFC_PROPERTY_SET_FILL,
            Class::IfcPropertySingleValue => &IFC_PROPERTY_SINGLE_VALUE_FILL,
            Class::IfcPropertyTableValue => &IFC_PROPERTY_TABLE_VALUE_FILL,
            Class::IfcProtectiveDeviceType => &IFC_PROTECTIVE_DEVICE_TYPE_FILL,
            Class::IfcProxy => &IFC_PROXY_FILL,
            Class::IfcPumpType => &IFC_PUMP_TYPE_FILL,
            Class::IfcRadiusDimension => &IFC_RADIUS_DIMENSION_FILL,
            Class::IfcRailing => &IFC_RAILING_FILL,
            Class::IfcRailingType => &IFC_RAILING_TYPE_FILL,
            Class::IfcRamp => &IFC_RAMP_FILL,
            Class::IfcRampFlight => &IFC_RAMP_FLIGHT_FILL,
            Class::IfcRampFlightType => &IFC_RAMP_FLIGHT_TYPE_FILL,
            Class::IfcRationalBezierCurve => &IFC_RATIONAL_BEZIER_CURVE_FILL,
            Class::IfcRectangleProfileDef => &IFC_RECTANGLE_PROFILE_DEF_FILL,
            Class::IfcRectangleHollowProfileDef => &IFC_RECTANGLE_HOLLOW_PROFILE_DEF_FILL,
            Class::IfcRectangularPyramid => &IFC_RECTANGULAR_PYRAMID_FILL,
            Class::IfcRectangularTrimmedSurface => &IFC_RECTANGULAR_TRIMMED_SURFACE_FILL,
            Class::IfcReinforcingElement => &IFC_REINFORCING_ELEMENT_FILL,
            Class::IfcReinforcingBar => &IFC_REINFORCING_BAR_FILL,
            Class::IfcReinforcingMesh => &IFC_REINFORCING_MESH_FILL,
            Class::IfcRelationship => &IFC_RELATIONSHIP_FILL,
            Class::IfcRelDecomposes => &IFC_REL_DECOMPOSES_FILL,
            Class::IfcRelAggregates => &IFC_REL_AGGREGATES_FILL,
            Class::IfcRelConnects => &IFC_REL_CONNECTS_FILL,
            Class::IfcRelContainedInSpatialStructure => &IFC_REL_CONTAINED_IN_SPATIAL_STRUCTURE_FILL,
            Class::IfcRelDefines => &IFC_REL_DEFINES_FILL,
            Class::IfcRelDefinesByProperties => &IFC_REL_DEFINES_BY_PROPERTIES_FILL,
            Class::IfcRelFillsElement => &IFC_REL_FILLS_ELEMENT_FILL,
            Class::IfcRelOverridesProperties => &IFC_REL_OVERRIDES_PROPERTIES_FILL,
            Class::IfcRelVoidsElement => &IFC_REL_VOIDS_ELEMENT_FILL,
            Class::IfcRepresentation => &IFC_REPRESENTATION_FILL,
            Class::IfcRepresentationMap => &IFC_REPRESENTATION_MAP_FILL,
            Class::IfcRevolvedAreaSolid => &IFC_REVOLVED_AREA_SOLID_FILL,
            Class::IfcRightCircularCone => &IFC_RIGHT_CIRCULAR_CONE_FILL,
            Class::IfcRightCircularCylinder => &IFC_RIGHT_CIRCULAR_CYLINDER_FILL,
            Class::IfcRoof => &IFC_ROOF_FILL,
            Class::IfcRoundedEdgeFeature => &IFC_ROUNDED_EDGE_FEATURE_FILL,
            Class::IfcRoundedRectangleProfileDef => &IFC_ROUNDED_RECTANGLE_PROFILE_DEF_FILL,
            Class::IfcSIUnit => &IFC_SI_UNIT_FILL,
            Class::IfcSanitaryTerminalType => &IFC_SANITARY_TERMINAL_TYPE_FILL,
            Class::IfcScheduleTimeControl => &IFC_SCHEDULE_TIME_CONTROL_FILL,
            Class::IfcSectionedSpine => &IFC_SECTIONED_SPINE_FILL,
            Class::IfcSensorType => &IFC_SENSOR_TYPE_FILL,
            Class::IfcServiceLife => &IFC_SERVICE_LIFE_FILL,
            Class::IfcShapeModel => &IFC_SHAPE_MODEL_FILL,
            Class::IfcShapeRepresentation => &IFC_SHAPE_REPRESENTATION_FILL,
            Class::IfcShellBasedSurfaceModel => &IFC_SHELL_BASED_SURFACE_MODEL_FILL,
            Class::IfcSite => &IFC_SITE_FILL,
            Class::IfcSlab => &IFC_SLAB_FILL,
            Class::IfcSlabType => &IFC_SLAB_TYPE_FILL,
            Class::IfcSpace => &IFC_SPACE_FILL,
            Class::IfcSpaceHeaterType => &IFC_SPACE_HEATER_TYPE_FILL,
            Class::IfcSpaceProgram => &IFC_SPACE_PROGRAM_FILL,
            Class::IfcSpatialStructureElementType => &IFC_SPATIAL_STRUCTURE_ELEMENT_TYPE_FILL,
            Class::IfcSpaceType => &IFC_SPACE_TYPE_FILL,
            Class::IfcSphere => &IFC_SPHERE_FILL,
            Class::IfcStackTerminalType => &IFC_STACK_TERMINAL_TYPE_FILL,
            Class::IfcStair => &IFC_STAIR_FILL,
            Class::IfcStairFlight => &IFC_STAIR_FLIGHT_FILL,
            Class::IfcStairFlightType => &IFC_STAIR_FLIGHT_TYPE_FILL,
            Class::IfcStructuralActivity => &IFC_STRUCTURAL_ACTIVITY_FILL,
            Class::IfcStructuralAction => &IFC_STRUCTURAL_ACTION_FILL,
            Class::IfcStructuralAnalysisModel => &IFC_STRUCTURAL_ANALYSIS_MODEL_FILL,
            Class::IfcStructuralItem => &IFC_STRUCTURAL_ITEM_FILL,
            Class::IfcStructuralConnection => &IFC_STRUCTURAL_CONNECTION_FILL,
            Class::IfcStructuralCurveConnection => &IFC_STRUCTURAL_CURVE_CONNECTION_FILL,
            Class::IfcStructuralMember => &IFC_STRUCTURAL_MEMBER_FILL,
            Class::IfcStructuralCurveMember => &IFC_STRUCTURAL_CURVE_MEMBER_FILL,
            Class::IfcStructuralCurveMemberVarying => &IFC_STRUCTURAL_CURVE_MEMBER_VARYING_FILL,
            Class::IfcStructuralLinearAction => &IFC_STRUCTURAL_LINEAR_ACTION_FILL,
            Class::IfcStructuralLinearActionVarying => &IFC_STRUCTURAL_LINEAR_ACTION_VARYING_FILL,
            Class::IfcStructuralLoadGroup => &IFC_STRUCTURAL_LOAD_GROUP_FILL,
            Class::IfcStructuralPlanarAction => &IFC_STRUCTURAL_PLANAR_ACTION_FILL,
            Class::IfcStructuralPlanarActionVarying => &IFC_STRUCTURAL_PLANAR_ACTION_VARYING_FILL,
            Class::IfcStructuralPointAction => &IFC_STRUCTURAL_POINT_ACTION_FILL,
            Class::IfcStructuralPointConnection => &IFC_STRUCTURAL_POINT_CONNECTION_FILL,
            Class::IfcStructuralReaction => &IFC_STRUCTURAL_REACTION_FILL,
            Class::IfcStructuralPointReaction => &IFC_STRUCTURAL_POINT_REACTION_FILL,
            Class::IfcStructuralResultGroup => &IFC_STRUCTURAL_RESULT_GROUP_FILL,
            Class::IfcStructuralSurfaceConnection => &IFC_STRUCTURAL_SURFACE_CONNECTION_FILL,
            Class::IfcStructuralSurfaceMember => &IFC_STRUCTURAL_SURFACE_MEMBER_FILL,
            Class::IfcStructuralSurfaceMemberVarying => &IFC_STRUCTURAL_SURFACE_MEMBER_VARYING_FILL,
            Class::IfcStructuredDimensionCallout => &IFC_STRUCTURED_DIMENSION_CALLOUT_FILL,
            Class::IfcStyleModel => &IFC_STYLE_MODEL_FILL,
            Class::IfcStyledRepresentation => &IFC_STYLED_REPRESENTATION_FILL,
            Class::IfcSubContractResource => &IFC_SUB_CONTRACT_RESOURCE_FILL,
            Class::IfcSubedge => &IFC_SUBEDGE_FILL,
            Class::IfcSurfaceCurveSweptAreaSolid => &IFC_SURFACE_CURVE_SWEPT_AREA_SOLID_FILL,
            Class::IfcSweptSurface => &IFC_SWEPT_SURFACE_FILL,
            Class::IfcSurfaceOfLinearExtrusion => &IFC_SURFACE_OF_LINEAR_EXTRUSION_FILL,
            Class::IfcSurfaceOfRevolution => &IFC_SURFACE_OF_REVOLUTION_FILL,
            Class::IfcSurfaceStyle => &IFC_SURFACE_STYLE_FILL,
            Class::IfcSurfaceStyleShading => &IFC_SURFACE_STYLE_SHADING_FILL,
            Class::IfcSurfaceStyleRendering => &IFC_SURFACE_STYLE_RENDERING_FILL,
            Class::IfcSurfaceStyleWithTextures => &IFC_SURFACE_STYLE_WITH_TEXTURES_FILL,
            Class::IfcSweptDiskSolid => &IFC_SWEPT_DISK_SOLID_FILL,
            Class::IfcSwitchingDeviceType => &IFC_SWITCHING_DEVICE_TYPE_FILL,
            Class::IfcSystemFurnitureElementType => &IFC_SYSTEM_FURNITURE_ELEMENT_TYPE_FILL,
            Class::IfcTShapeProfileDef => &IFC_T_SHAPE_PROFILE_DEF_FILL,
            Class::IfcTankType => &IFC_TANK_TYPE_FILL,
            Class::IfcTendon => &IFC_TENDON_FILL,
            Class::IfcTendonAnchor => &IFC_TENDON_ANCHOR_FILL,
            Class::IfcTextLiteral => &IFC_TEXT_LITERAL_FILL,
            Class::IfcTextLiteralWithExtent => &IFC_TEXT_LITERAL_WITH_EXTENT_FILL,
            Class::IfcTimeSeriesSchedule => &IFC_TIME_SERIES_SCHEDULE_FILL,
            Class::IfcTopologyRepresentation => &IFC_TOPOLOGY_REPRESENTATION_FILL,
            Class::IfcTransformerType => &IFC_TRANSFORMER_TYPE_FILL,
            Class::IfcTransportElement => &IFC_TRANSPORT_ELEMENT_FILL,
            Class::IfcTransportElementType => &IFC_TRANSPORT_ELEMENT_TYPE_FILL,
            Class::IfcTrapeziumProfileDef => &IFC_TRAPEZIUM_PROFILE_DEF_FILL,
            Class::IfcTrimmedCurve => &IFC_TRIMMED_CURVE_FILL,
            Class::IfcTubeBundleType => &IFC_TUBE_BUNDLE_TYPE_FILL,
            Class::IfcTwoDirectionRepeatFactor => &IFC_TWO_DIRECTION_REPEAT_FACTOR_FILL,
            Class::IfcUShapeProfileDef => &IFC_U_SHAPE_PROFILE_DEF_FILL,
            Class::IfcUnitAssignment => &IFC_UNIT_ASSIGNMENT_FILL,
            Class::IfcUnitaryEquipmentType => &IFC_UNITARY_EQUIPMENT_TYPE_FILL,
            Class::IfcValveType => &IFC_VALVE_TYPE_FILL,
            Class::IfcVector => &IFC_VECTOR_FILL,
            Class::IfcVertex => &IFC_VERTEX_FILL,
            Class::IfcVertexLoop => &IFC_VERTEX_LOOP_FILL,
            Class::IfcVertexPoint => &IFC_VERTEX_POINT_FILL,
            Class::IfcVibrationIsolatorType => &IFC_VIBRATION_ISOLATOR_TYPE_FILL,
            Class::IfcVirtualElement => &IFC_VIRTUAL_ELEMENT_FILL,
            Class::IfcWall => &IFC_WALL_FILL,
            Class::IfcWallStandardCase => &IFC_WALL_STANDARD_CASE_FILL,
            Class::IfcWallType => &IFC_WALL_TYPE_FILL,
            Class::IfcWasteTerminalType => &IFC_WASTE_TERMINAL_TYPE_FILL,
            Class::IfcWindow => &IFC_WINDOW_FILL,
            Class::IfcWindowStyle => &IFC_WINDOW_STYLE_FILL,
            Class::IfcWorkControl => &IFC_WORK_CONTROL_FILL,
            Class::IfcWorkPlan => &IFC_WORK_PLAN_FILL,
            Class::IfcWorkSchedule => &IFC_WORK_SCHEDULE_FILL,
            Class::IfcZShapeProfileDef => &IFC_Z_SHAPE_PROFILE_DEF_FILL,
            Class::IfcZone => &IFC_ZONE_FILL,
        }
    }
}

const IFC_REPRESENTATION_ITEM_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_GEOMETRIC_REPRESENTATION_ITEM_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CURVE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_BOUNDED_CURVE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_COMPOSITE_CURVE_FILL: ClassFill = ClassFill {
    required: Some(2),
    fields: &[
        FieldFill { name: "Segments", slot: 0, field_type: FieldType::List(&FieldType::Entity, 1, 0), maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "SelfIntersect", slot: 1, field_type: FieldType::Select, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC2_D_COMPOSITE_CURVE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ROOT_FILL: ClassFill = ClassFill {
    required: Some(4),
    fields: &[
        FieldFill { name: "GlobalId", slot: 0, field_type: FieldType::Text, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "OwnerHistory", slot: 1, field_type: FieldType::Entity, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "Name", slot: 2, field_type: FieldType::Text, maybe: true, derived_check: true, unset_check: true },
        FieldFill { name: "Description", slot: 3, field_type: FieldType::Text, maybe: true, derived_check: true, unset_check: true },
    ],
};
const IFC_OBJECT_DEFINITION_FILL: ClassFill = ClassFill { required: Some(4), fields: &[] };
const IFC_OBJECT_FILL: ClassFill = ClassFill {
    required: Some(5),
    fields: &[
        FieldFill { name: "ObjectType", slot: 4, field_type: FieldType::Text, maybe: true, derived_check: true, unset_check: true },
    ],
};
const IFC_CONTROL_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ACTION_REQUEST_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ACTOR_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_TYPE_OBJECT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_TYPE_PRODUCT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ELEMENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DISTRIBUTION_ELEMENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DISTRIBUTION_CONTROL_ELEMENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ACTUATOR_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DISTRIBUTION_FLOW_ELEMENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FLOW_CONTROLLER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_AIR_TERMINAL_BOX_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FLOW_TERMINAL_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_AIR_TERMINAL_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ENERGY_CONVERSION_DEVICE_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_AIR_TO_AIR_HEAT_RECOVERY_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ALARM_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DRAUGHTING_CALLOUT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DIMENSION_CURVE_DIRECTED_CALLOUT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ANGULAR_DIMENSION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PRODUCT_FILL: ClassFill = ClassFill {
    required: Some(7),
    fields: &[
        FieldFill { name: "ObjectPlacement", slot: 5, field_type: FieldType::Entity, maybe: true, derived_check: true, unset_check: true },
        FieldFill { name: "Representation", slot: 6, field_type: FieldType::Entity, maybe: true, derived_check: true, unset_check: true },
    ],
};
const IFC_ANNOTATION_FILL: ClassFill = ClassFill { required: Some(7), fields: &[] };
const IFC_STYLED_ITEM_FILL: ClassFill = ClassFill {
    required: Some(3),
    fields: &[
        FieldFill { name: "Item", slot: 0, field_type: FieldType::Entity, maybe: true, derived_check: true, unset_check: true },
        FieldFill { name: "Styles", slot: 1, field_type: FieldType::List(&FieldType::Entity, 1, 0), maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "Name", slot: 2, field_type: FieldType::Text, maybe: true, derived_check: true, unset_check: true },
    ],
};
const IFC_ANNOTATION_OCCURRENCE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ANNOTATION_CURVE_OCCURRENCE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ANNOTATION_FILL_AREA_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ANNOTATION_FILL_AREA_OCCURRENCE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ANNOTATION_SURFACE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ANNOTATION_SURFACE_OCCURRENCE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ANNOTATION_SYMBOL_OCCURRENCE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ANNOTATION_TEXT_OCCURRENCE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PROFILE_DEF_FILL: ClassFill = ClassFill {
    required: Some(2),
    fields: &[
        FieldFill { name: "ProfileType", slot: 0, field_type: FieldType::Text, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "ProfileName", slot: 1, field_type: FieldType::Text, maybe: true, derived_check: true, unset_check: true },
    ],
};
const IFC_ARBITRARY_CLOSED_PROFILE_DEF_FILL: ClassFill = ClassFill {
    required: Some(3),
    fields: &[
        FieldFill { name: "OuterCurve", slot: 2, field_type: FieldType::Entity, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_ARBITRARY_OPEN_PROFILE_DEF_FILL: ClassFill = ClassFill {
    required: Some(3),
    fields: &[
        FieldFill { name: "Curve", slot: 2, field_type: FieldType::Entity, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_ARBITRARY_PROFILE_DEF_WITH_VOIDS_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_GROUP_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ASSET_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PARAMETERIZED_PROFILE_DEF_FILL: ClassFill = ClassFill {
    required: Some(3),
    fields: &[
        FieldFill { name: "Position", slot: 2, field_type: FieldType::Entity, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_I_SHAPE_PROFILE_DEF_FILL: ClassFill = ClassFill {
    required: Some(8),
    fields: &[
        FieldFill { name: "OverallWidth", slot: 3, field_type: FieldType::Real, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "OverallDepth", slot: 4, field_type: FieldType::Real, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "WebThickness", slot: 5, field_type: FieldType::Real, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "FlangeThickness", slot: 6, field_type: FieldType::Real, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "FilletRadius", slot: 7, field_type: FieldType::Real, maybe: true, derived_check: true, unset_check: true },
    ],
};
const IFC_ASYMMETRIC_I_SHAPE_PROFILE_DEF_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PLACEMENT_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "Location", slot: 0, field_type: FieldType::Entity, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_AXIS1_PLACEMENT_FILL: ClassFill = ClassFill {
    required: Some(2),
    fields: &[
        FieldFill { name: "Axis", slot: 1, field_type: FieldType::Entity, maybe: true, derived_check: false, unset_check: true },
    ],
};
const IFC_AXIS2_PLACEMENT2_D_FILL: ClassFill = ClassFill {
    required: Some(2),
    fields: &[
        FieldFill { name: "RefDirection", slot: 1, field_type: FieldType::Entity, maybe: true, derived_check: false, unset_check: true },
    ],
};
const IFC_AXIS2_PLACEMENT3_D_FILL: ClassFill = ClassFill {
    required: Some(3),
    fields: &[
        FieldFill { name: "Axis", slot: 1, field_type: FieldType::Entity, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "RefDirection", slot: 2, field_type: FieldType::Entity, maybe: true, derived_check: false, unset_check: true },
    ],
};
const IFC_B_SPLINE_CURVE_FILL: ClassFill = ClassFill {
    required: Some(5),
    fields: &[
        FieldFill { name: "Degree", slot: 0, field_type: FieldType::Integer, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "ControlPointsList", slot: 1, field_type: FieldType::List(&FieldType::Entity, 2, 0), maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "CurveForm", slot: 2, field_type: FieldType::Text, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "ClosedCurve", slot: 3, field_type: FieldType::Select, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "SelfIntersect", slot: 4, field_type: FieldType::Select, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_ELEMENT_FILL: ClassFill = ClassFill {
    required: Some(8),
    fields: &[
        FieldFill { name: "Tag", slot: 7, field_type: FieldType::Text, maybe: true, derived_check: true, unset_check: true },
    ],
};
const IFC_BUILDING_ELEMENT_FILL: ClassFill = ClassFill { required: Some(8), fields: &[] };
const IFC_BEAM_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_BUILDING_ELEMENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_BEAM_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_BEZIER_CURVE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CSG_PRIMITIVE3_D_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_BLOCK_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_BOILER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_BOOLEAN_RESULT_FILL: ClassFill = ClassFill {
    required: Some(3),
    fields: &[
        FieldFill { name: "Operator", slot: 0, field_type: FieldType::Text, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "FirstOperand", slot: 1, field_type: FieldType::Select, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "SecondOperand", slot: 2, field_type: FieldType::Select, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_BOOLEAN_CLIPPING_RESULT_FILL: ClassFill = ClassFill { required: Some(3), fields: &[] };
const IFC_SURFACE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_BOUNDED_SURFACE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_BOUNDING_BOX_FILL: ClassFill = ClassFill {
    required: Some(4),
    fields: &[
        FieldFill { name: "Corner", slot: 0, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "XDim", slot: 1, field_type: FieldType::Real, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "YDim", slot: 2, field_type: FieldType::Real, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "ZDim", slot: 3, field_type: FieldType::Real, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_HALF_SPACE_SOLID_FILL: ClassFill = ClassFill {
    required: Some(2),
    fields: &[
        FieldFill { name: "BaseSurface", slot: 0, field_type: FieldType::Entity, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "AgreementFlag", slot: 1, field_type: FieldType::Text, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_BOXED_HALF_SPACE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SPATIAL_STRUCTURE_ELEMENT_FILL: ClassFill = ClassFill {
    required: Some(9),
    fields: &[
        FieldFill { name: "LongName", slot: 7, field_type: FieldType::Text, maybe: true, derived_check: true, unset_check: true },
        FieldFill { name: "CompositionType", slot: 8, field_type: FieldType::Text, maybe: false, derived_check: true, unset_check: true },
    ],
};
const IFC_BUILDING_FILL: ClassFill = ClassFill {
    required: Some(12),
    fields: &[
        FieldFill { name: "ElevationOfRefHeight", slot: 9, field_type: FieldType::Real, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "ElevationOfTerrain", slot: 10, field_type: FieldType::Real, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "BuildingAddress", slot: 11, field_type: FieldType::Entity, maybe: true, derived_check: false, unset_check: true },
    ],
};
const IFC_BUILDING_ELEMENT_COMPONENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_BUILDING_ELEMENT_PART_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_BUILDING_ELEMENT_PROXY_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_BUILDING_ELEMENT_PROXY_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_BUILDING_STOREY_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_C_SHAPE_PROFILE_DEF_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FLOW_FITTING_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CABLE_CARRIER_FITTING_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FLOW_SEGMENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CABLE_CARRIER_SEGMENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CABLE_SEGMENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_POINT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CARTESIAN_POINT_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "Coordinates", slot: 0, field_type: FieldType::List(&FieldType::Real, 1, 3), maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_CARTESIAN_TRANSFORMATION_OPERATOR_FILL: ClassFill = ClassFill {
    required: Some(4),
    fields: &[
        FieldFill { name: "Axis1", slot: 0, field_type: FieldType::Entity, maybe: true, derived_check: true, unset_check: true },
        FieldFill { name: "Axis2", slot: 1, field_type: FieldType::Entity, maybe: true, derived_check: true, unset_check: true },
        FieldFill { name: "LocalOrigin", slot: 2, field_type: FieldType::Entity, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "Scale", slot: 3, field_type: FieldType::Real, maybe: true, derived_check: true, unset_check: true },
    ],
};
const IFC_CARTESIAN_TRANSFORMATION_OPERATOR2_D_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CARTESIAN_TRANSFORMATION_OPERATOR2_DNON_UNIFORM_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CARTESIAN_TRANSFORMATION_OPERATOR3_D_FILL: ClassFill = ClassFill {
    required: Some(5),
    fields: &[
        FieldFill { name: "Axis3", slot: 4, field_type: FieldType::Entity, maybe: true, derived_check: true, unset_check: true },
    ],
};
const IFC_CARTESIAN_TRANSFORMATION_OPERATOR3_DNON_UNIFORM_FILL: ClassFill = ClassFill {
    required: Some(7),
    fields: &[
        FieldFill { name: "Scale2", slot: 5, field_type: FieldType::Real, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "Scale3", slot: 6, field_type: FieldType::Real, maybe: true, derived_check: false, unset_check: true },
    ],
};
const IFC_CENTER_LINE_PROFILE_DEF_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FEATURE_ELEMENT_FILL: ClassFill = ClassFill { required: Some(8), fields: &[] };
const IFC_FEATURE_ELEMENT_SUBTRACTION_FILL: ClassFill = ClassFill { required: Some(8), fields: &[] };
const IFC_EDGE_FEATURE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CHAMFER_EDGE_FEATURE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CHILLER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CONIC_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "Position", slot: 0, field_type: FieldType::Select, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_CIRCLE_FILL: ClassFill = ClassFill {
    required: Some(2),
    fields: &[
        FieldFill { name: "Radius", slot: 1, field_type: FieldType::Real, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_CIRCLE_PROFILE_DEF_FILL: ClassFill = ClassFill {
    required: Some(4),
    fields: &[
        FieldFill { name: "Radius", slot: 3, field_type: FieldType::Real, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_CIRCLE_HOLLOW_PROFILE_DEF_FILL: ClassFill = ClassFill {
    required: Some(5),
    fields: &[
        FieldFill { name: "WallThickness", slot: 4, field_type: FieldType::Real, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_TOPOLOGICAL_REPRESENTATION_ITEM_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CONNECTED_FACE_SET_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "CfsFaces", slot: 0, field_type: FieldType::List(&FieldType::Entity, 1, 0), maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_CLOSED_SHELL_FILL: ClassFill = ClassFill { required: Some(1), fields: &[] };
const IFC_COIL_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_COLOUR_SPECIFICATION_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "Name", slot: 0, field_type: FieldType::Text, maybe: true, derived_check: true, unset_check: true },
    ],
};
const IFC_COLOUR_RGB_FILL: ClassFill = ClassFill {
    required: Some(4),
    fields: &[
        FieldFill { name: "Red", slot: 1, field_type: FieldType::Real, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "Green", slot: 2, field_type: FieldType::Real, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "Blue", slot: 3, field_type: FieldType::Real, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_COLUMN_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_COLUMN_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PROPERTY_FILL: ClassFill = ClassFill {
    required: Some(2),
    fields: &[
        FieldFill { name: "Name", slot: 0, field_type: FieldType::Text, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "Description", slot: 1, field_type: FieldType::Text, maybe: true, derived_check: true, unset_check: true },
    ],
};
const IFC_COMPLEX_PROPERTY_FILL: ClassFill = ClassFill {
    required: Some(4),
    fields: &[
        FieldFill { name: "UsageName", slot: 2, field_type: FieldType::Text, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "HasProperties", slot: 3, field_type: FieldType::List(&FieldType::Entity, 1, 0), maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_COMPOSITE_CURVE_SEGMENT_FILL: ClassFill = ClassFill {
    required: Some(3),
    fields: &[
        FieldFill { name: "Transition", slot: 0, field_type: FieldType::Text, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "SameSense", slot: 1, field_type: FieldType::Text, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "ParentCurve", slot: 2, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_COMPOSITE_PROFILE_DEF_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FLOW_MOVING_DEVICE_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_COMPRESSOR_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CONDENSER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CONDITION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CONDITION_CRITERION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_RESOURCE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CONSTRUCTION_RESOURCE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CONSTRUCTION_EQUIPMENT_RESOURCE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CONSTRUCTION_MATERIAL_RESOURCE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CONSTRUCTION_PRODUCT_RESOURCE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_NAMED_UNIT_FILL: ClassFill = ClassFill {
    required: Some(2),
    fields: &[
        FieldFill { name: "Dimensions", slot: 0, field_type: FieldType::Entity, maybe: false, derived_check: true, unset_check: true },
        FieldFill { name: "UnitType", slot: 1, field_type: FieldType::Text, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_CONTEXT_DEPENDENT_UNIT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CONTROLLER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CONVERSION_BASED_UNIT_FILL: ClassFill = ClassFill {
    required: Some(4),
    fields: &[
        FieldFill { name: "Name", slot: 2, field_type: FieldType::Text, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "ConversionFactor", slot: 3, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_COOLED_BEAM_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_COOLING_TOWER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_COST_ITEM_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_COST_SCHEDULE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_COVERING_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_COVERING_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CRANE_RAIL_A_SHAPE_PROFILE_DEF_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CRANE_RAIL_F_SHAPE_PROFILE_DEF_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CREW_RESOURCE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SOLID_MODEL_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CSG_SOLID_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CURTAIN_WALL_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CURTAIN_WALL_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_CURVE_BOUNDED_PLANE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PRESENTATION_STYLE_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "Name", slot: 0, field_type: FieldType::Text, maybe: true, derived_check: true, unset_check: true },
    ],
};
const IFC_DAMPER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DEFINED_SYMBOL_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DERIVED_PROFILE_DEF_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DIAMETER_DIMENSION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DIMENSION_CURVE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_TERMINATOR_SYMBOL_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DIMENSION_CURVE_TERMINATOR_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DIRECTION_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "DirectionRatios", slot: 0, field_type: FieldType::List(&FieldType::Real, 2, 3), maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_ELEMENT_COMPONENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DISCRETE_ACCESSORY_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ELEMENT_COMPONENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DISCRETE_ACCESSORY_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DISTRIBUTION_ELEMENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DISTRIBUTION_FLOW_ELEMENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DISTRIBUTION_CHAMBER_ELEMENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DISTRIBUTION_CHAMBER_ELEMENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DISTRIBUTION_CONTROL_ELEMENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PORT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DISTRIBUTION_PORT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DOOR_FILL: ClassFill = ClassFill {
    required: Some(10),
    fields: &[
        FieldFill { name: "OverallHeight", slot: 8, field_type: FieldType::Real, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "OverallWidth", slot: 9, field_type: FieldType::Real, maybe: true, derived_check: false, unset_check: true },
    ],
};
const IFC_PROPERTY_DEFINITION_FILL: ClassFill = ClassFill { required: Some(4), fields: &[] };
const IFC_PROPERTY_SET_DEFINITION_FILL: ClassFill = ClassFill { required: Some(4), fields: &[] };
const IFC_DOOR_STYLE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DUCT_FITTING_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DUCT_SEGMENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FLOW_TREATMENT_DEVICE_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_DUCT_SILENCER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_EDGE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_EDGE_CURVE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_LOOP_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_EDGE_LOOP_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ELECTRIC_APPLIANCE_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FLOW_CONTROLLER_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ELECTRIC_DISTRIBUTION_POINT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FLOW_STORAGE_DEVICE_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ELECTRIC_FLOW_STORAGE_DEVICE_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ELECTRIC_GENERATOR_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ELECTRIC_HEATER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ELECTRIC_MOTOR_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ELECTRIC_TIME_CONTROL_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SYSTEM_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ELECTRICAL_CIRCUIT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ELECTRICAL_ELEMENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ELEMENT_ASSEMBLY_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ELEMENT_QUANTITY_FILL: ClassFill = ClassFill {
    required: Some(6),
    fields: &[
        FieldFill { name: "MethodOfMeasurement", slot: 4, field_type: FieldType::Text, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "Quantities", slot: 5, field_type: FieldType::List(&FieldType::Entity, 1, 0), maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_ELEMENTARY_SURFACE_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "Position", slot: 0, field_type: FieldType::Entity, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_ELLIPSE_FILL: ClassFill = ClassFill {
    required: Some(3),
    fields: &[
        FieldFill { name: "SemiAxis1", slot: 1, field_type: FieldType::Real, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "SemiAxis2", slot: 2, field_type: FieldType::Real, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_ELLIPSE_PROFILE_DEF_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ENERGY_CONVERSION_DEVICE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_EQUIPMENT_ELEMENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_EQUIPMENT_STANDARD_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_EVAPORATIVE_COOLER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_EVAPORATOR_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SWEPT_AREA_SOLID_FILL: ClassFill = ClassFill {
    required: Some(2),
    fields: &[
        FieldFill { name: "SweptArea", slot: 0, field_type: FieldType::Entity, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "Position", slot: 1, field_type: FieldType::Entity, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_EXTRUDED_AREA_SOLID_FILL: ClassFill = ClassFill {
    required: Some(4),
    fields: &[
        FieldFill { name: "ExtrudedDirection", slot: 2, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "Depth", slot: 3, field_type: FieldType::Real, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_FACE_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "Bounds", slot: 0, field_type: FieldType::List(&FieldType::Entity, 1, 0), maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_FACE_BASED_SURFACE_MODEL_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "FbsmFaces", slot: 0, field_type: FieldType::List(&FieldType::Entity, 1, 0), maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_FACE_BOUND_FILL: ClassFill = ClassFill {
    required: Some(2),
    fields: &[
        FieldFill { name: "Bound", slot: 0, field_type: FieldType::Entity, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "Orientation", slot: 1, field_type: FieldType::Text, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_FACE_OUTER_BOUND_FILL: ClassFill = ClassFill { required: Some(2), fields: &[] };
const IFC_FACE_SURFACE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_MANIFOLD_SOLID_BREP_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "Outer", slot: 0, field_type: FieldType::Entity, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_FACETED_BREP_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FACETED_BREP_WITH_VOIDS_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FAN_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FASTENER_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FASTENER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FEATURE_ELEMENT_ADDITION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FILL_AREA_STYLE_HATCHING_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FILL_AREA_STYLE_TILE_SYMBOL_WITH_STYLE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FILL_AREA_STYLE_TILES_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FILTER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FIRE_SUPPRESSION_TERMINAL_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FLOW_FITTING_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FLOW_INSTRUMENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FLOW_METER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FLOW_MOVING_DEVICE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FLOW_SEGMENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FLOW_STORAGE_DEVICE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FLOW_TERMINAL_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FLOW_TREATMENT_DEVICE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FOOTING_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FURNISHING_ELEMENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FURNISHING_ELEMENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FURNITURE_STANDARD_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_FURNITURE_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_GAS_TERMINAL_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_GEOMETRIC_SET_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_GEOMETRIC_CURVE_SET_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_REPRESENTATION_CONTEXT_FILL: ClassFill = ClassFill {
    required: Some(2),
    fields: &[
        FieldFill { name: "ContextIdentifier", slot: 0, field_type: FieldType::Text, maybe: true, derived_check: true, unset_check: true },
        FieldFill { name: "ContextType", slot: 1, field_type: FieldType::Text, maybe: true, derived_check: true, unset_check: true },
    ],
};
const IFC_GEOMETRIC_REPRESENTATION_CONTEXT_FILL: ClassFill = ClassFill {
    required: Some(6),
    fields: &[
        FieldFill { name: "CoordinateSpaceDimension", slot: 2, field_type: FieldType::Integer, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "Precision", slot: 3, field_type: FieldType::Real, maybe: true, derived_check: true, unset_check: true },
        FieldFill { name: "WorldCoordinateSystem", slot: 4, field_type: FieldType::Select, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "TrueNorth", slot: 5, field_type: FieldType::Entity, maybe: true, derived_check: true, unset_check: true },
    ],
};
const IFC_GEOMETRIC_REPRESENTATION_SUB_CONTEXT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_GRID_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_OBJECT_PLACEMENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_GRID_PLACEMENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_HEAT_EXCHANGER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_HUMIDIFIER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_INVENTORY_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_JUNCTION_BOX_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_L_SHAPE_PROFILE_DEF_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_LABOR_RESOURCE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_LAMP_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_LIGHT_FIXTURE_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_LIGHT_SOURCE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_LIGHT_SOURCE_AMBIENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_LIGHT_SOURCE_DIRECTIONAL_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_LIGHT_SOURCE_GONIOMETRIC_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_LIGHT_SOURCE_POSITIONAL_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_LIGHT_SOURCE_SPOT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_LINE_FILL: ClassFill = ClassFill {
    required: Some(2),
    fields: &[
        FieldFill { name: "Pnt", slot: 0, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "Dir", slot: 1, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_LINEAR_DIMENSION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_LOCAL_PLACEMENT_FILL: ClassFill = ClassFill {
    required: Some(2),
    fields: &[
        FieldFill { name: "PlacementRelTo", slot: 0, field_type: FieldType::Entity, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "RelativePlacement", slot: 1, field_type: FieldType::Select, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_MAPPED_ITEM_FILL: ClassFill = ClassFill {
    required: Some(2),
    fields: &[
        FieldFill { name: "MappingSource", slot: 0, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "MappingTarget", slot: 1, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_PRODUCT_REPRESENTATION_FILL: ClassFill = ClassFill {
    required: Some(3),
    fields: &[
        FieldFill { name: "Name", slot: 0, field_type: FieldType::Text, maybe: true, derived_check: true, unset_check: true },
        FieldFill { name: "Description", slot: 1, field_type: FieldType::Text, maybe: true, derived_check: true, unset_check: true },
        FieldFill { name: "Representations", slot: 2, field_type: FieldType::List(&FieldType::Entity, 1, 0), maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_MATERIAL_DEFINITION_REPRESENTATION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_MEASURE_WITH_UNIT_FILL: ClassFill = ClassFill {
    required: Some(2),
    fields: &[
        FieldFill { name: "ValueComponent", slot: 0, field_type: FieldType::Select, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "UnitComponent", slot: 1, field_type: FieldType::Select, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_MECHANICAL_FASTENER_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_MECHANICAL_FASTENER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_MEMBER_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_MEMBER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_MOTOR_CONNECTION_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PROCESS_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_TASK_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_MOVE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_OCCUPANT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_OFFSET_CURVE2_D_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_OFFSET_CURVE3_D_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ONE_DIRECTION_REPEAT_FACTOR_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_OPEN_SHELL_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_OPENING_ELEMENT_FILL: ClassFill = ClassFill { required: Some(8), fields: &[] };
const IFC_ORDER_ACTION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ORIENTED_EDGE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_OUTLET_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PATH_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PERFORMANCE_HISTORY_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PERMIT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PILE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PIPE_FITTING_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PIPE_SEGMENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PLANAR_EXTENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PLANAR_BOX_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PLANE_FILL: ClassFill = ClassFill { required: Some(1), fields: &[] };
const IFC_PLATE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PLATE_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_POINT_ON_CURVE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_POINT_ON_SURFACE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_POLY_LOOP_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "Polygon", slot: 0, field_type: FieldType::List(&FieldType::Entity, 3, 0), maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_POLYGONAL_BOUNDED_HALF_SPACE_FILL: ClassFill = ClassFill {
    required: Some(4),
    fields: &[
        FieldFill { name: "Position", slot: 2, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "PolygonalBoundary", slot: 3, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_POLYLINE_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "Points", slot: 0, field_type: FieldType::List(&FieldType::Entity, 2, 0), maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_PRESENTATION_STYLE_ASSIGNMENT_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "Styles", slot: 0, field_type: FieldType::List(&FieldType::Select, 1, 0), maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_PROCEDURE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PRODUCT_DEFINITION_SHAPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PROJECT_FILL: ClassFill = ClassFill {
    required: Some(9),
    fields: &[
        FieldFill { name: "LongName", slot: 5, field_type: FieldType::Text, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "Phase", slot: 6, field_type: FieldType::Text, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "RepresentationContexts", slot: 7, field_type: FieldType::List(&FieldType::Entity, 1, 0), maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "UnitsInContext", slot: 8, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_PROJECT_ORDER_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PROJECT_ORDER_RECORD_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PROJECTION_CURVE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PROJECTION_ELEMENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SIMPLE_PROPERTY_FILL: ClassFill = ClassFill { required: Some(2), fields: &[] };
const IFC_PROPERTY_BOUNDED_VALUE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PROPERTY_ENUMERATED_VALUE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PROPERTY_LIST_VALUE_FILL: ClassFill = ClassFill {
    required: Some(4),
    fields: &[
        FieldFill { name: "ListValues", slot: 2, field_type: FieldType::List(&FieldType::Select, 1, 0), maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "Unit", slot: 3, field_type: FieldType::Select, maybe: true, derived_check: false, unset_check: true },
    ],
};
const IFC_PROPERTY_REFERENCE_VALUE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PROPERTY_SET_FILL: ClassFill = ClassFill {
    required: Some(5),
    fields: &[
        FieldFill { name: "HasProperties", slot: 4, field_type: FieldType::List(&FieldType::Entity, 1, 0), maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_PROPERTY_SINGLE_VALUE_FILL: ClassFill = ClassFill {
    required: Some(4),
    fields: &[
        FieldFill { name: "NominalValue", slot: 2, field_type: FieldType::Select, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "Unit", slot: 3, field_type: FieldType::Select, maybe: true, derived_check: false, unset_check: true },
    ],
};
const IFC_PROPERTY_TABLE_VALUE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PROTECTIVE_DEVICE_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PROXY_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_PUMP_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_RADIUS_DIMENSION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_RAILING_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_RAILING_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_RAMP_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_RAMP_FLIGHT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_RAMP_FLIGHT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_RATIONAL_BEZIER_CURVE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_RECTANGLE_PROFILE_DEF_FILL: ClassFill = ClassFill {
    required: Some(5),
    fields: &[
        FieldFill { name: "XDim", slot: 3, field_type: FieldType::Real, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "YDim", slot: 4, field_type: FieldType::Real, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_RECTANGLE_HOLLOW_PROFILE_DEF_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_RECTANGULAR_PYRAMID_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_RECTANGULAR_TRIMMED_SURFACE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_REINFORCING_ELEMENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_REINFORCING_BAR_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_REINFORCING_MESH_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_RELATIONSHIP_FILL: ClassFill = ClassFill { required: Some(4), fields: &[] };
const IFC_REL_DECOMPOSES_FILL: ClassFill = ClassFill {
    required: Some(6),
    fields: &[
        FieldFill { name: "RelatingObject", slot: 4, field_type: FieldType::Entity, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "RelatedObjects", slot: 5, field_type: FieldType::List(&FieldType::Entity, 1, 0), maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_REL_AGGREGATES_FILL: ClassFill = ClassFill { required: Some(6), fields: &[] };
const IFC_REL_CONNECTS_FILL: ClassFill = ClassFill { required: Some(4), fields: &[] };
const IFC_REL_CONTAINED_IN_SPATIAL_STRUCTURE_FILL: ClassFill = ClassFill {
    required: Some(6),
    fields: &[
        FieldFill { name: "RelatedElements", slot: 4, field_type: FieldType::List(&FieldType::Entity, 1, 0), maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "RelatingStructure", slot: 5, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_REL_DEFINES_FILL: ClassFill = ClassFill {
    required: Some(5),
    fields: &[
        FieldFill { name: "RelatedObjects", slot: 4, field_type: FieldType::List(&FieldType::Entity, 1, 0), maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_REL_DEFINES_BY_PROPERTIES_FILL: ClassFill = ClassFill {
    required: Some(6),
    fields: &[
        FieldFill { name: "RelatingPropertyDefinition", slot: 5, field_type: FieldType::Entity, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_REL_FILLS_ELEMENT_FILL: ClassFill = ClassFill {
    required: Some(6),
    fields: &[
        FieldFill { name: "RelatingOpeningElement", slot: 4, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "RelatedBuildingElement", slot: 5, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_REL_OVERRIDES_PROPERTIES_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_REL_VOIDS_ELEMENT_FILL: ClassFill = ClassFill {
    required: Some(6),
    fields: &[
        FieldFill { name: "RelatingBuildingElement", slot: 4, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "RelatedOpeningElement", slot: 5, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_REPRESENTATION_FILL: ClassFill = ClassFill {
    required: Some(4),
    fields: &[
        FieldFill { name: "ContextOfItems", slot: 0, field_type: FieldType::Entity, maybe: false, derived_check: true, unset_check: false },
        FieldFill { name: "RepresentationIdentifier", slot: 1, field_type: FieldType::Text, maybe: true, derived_check: true, unset_check: true },
        FieldFill { name: "RepresentationType", slot: 2, field_type: FieldType::Text, maybe: true, derived_check: true, unset_check: true },
        FieldFill { name: "Items", slot: 3, field_type: FieldType::List(&FieldType::Entity, 1, 0), maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_REPRESENTATION_MAP_FILL: ClassFill = ClassFill {
    required: Some(2),
    fields: &[
        FieldFill { name: "MappingOrigin", slot: 0, field_type: FieldType::Select, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "MappedRepresentation", slot: 1, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_REVOLVED_AREA_SOLID_FILL: ClassFill = ClassFill {
    required: Some(4),
    fields: &[
        FieldFill { name: "Axis", slot: 2, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "Angle", slot: 3, field_type: FieldType::Real, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_RIGHT_CIRCULAR_CONE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_RIGHT_CIRCULAR_CYLINDER_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ROOF_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ROUNDED_EDGE_FEATURE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ROUNDED_RECTANGLE_PROFILE_DEF_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SI_UNIT_FILL: ClassFill = ClassFill {
    required: Some(4),
    fields: &[
        FieldFill { name: "Prefix", slot: 2, field_type: FieldType::Text, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "Name", slot: 3, field_type: FieldType::Text, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_SANITARY_TERMINAL_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SCHEDULE_TIME_CONTROL_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SECTIONED_SPINE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SENSOR_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SERVICE_LIFE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SHAPE_MODEL_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SHAPE_REPRESENTATION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SHELL_BASED_SURFACE_MODEL_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "SbsmBoundary", slot: 0, field_type: FieldType::List(&FieldType::Select, 1, 0), maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_SITE_FILL: ClassFill = ClassFill {
    required: Some(14),
    fields: &[
        FieldFill { name: "RefLatitude", slot: 9, field_type: FieldType::List(&FieldType::Integer, 3, 3), maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "RefLongitude", slot: 10, field_type: FieldType::List(&FieldType::Integer, 3, 3), maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "RefElevation", slot: 11, field_type: FieldType::Real, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "LandTitleNumber", slot: 12, field_type: FieldType::Text, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "SiteAddress", slot: 13, field_type: FieldType::Entity, maybe: true, derived_check: false, unset_check: true },
    ],
};
const IFC_SLAB_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SLAB_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SPACE_FILL: ClassFill = ClassFill {
    required: Some(11),
    fields: &[
        FieldFill { name: "InteriorOrExteriorSpace", slot: 9, field_type: FieldType::Text, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "ElevationWithFlooring", slot: 10, field_type: FieldType::Real, maybe: true, derived_check: false, unset_check: true },
    ],
};
const IFC_SPACE_HEATER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SPACE_PROGRAM_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SPATIAL_STRUCTURE_ELEMENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SPACE_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SPHERE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STACK_TERMINAL_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STAIR_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STAIR_FLIGHT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STAIR_FLIGHT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_ACTIVITY_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_ACTION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_ANALYSIS_MODEL_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_ITEM_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_CONNECTION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_CURVE_CONNECTION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_MEMBER_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_CURVE_MEMBER_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_CURVE_MEMBER_VARYING_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_LINEAR_ACTION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_LINEAR_ACTION_VARYING_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_LOAD_GROUP_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_PLANAR_ACTION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_PLANAR_ACTION_VARYING_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_POINT_ACTION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_POINT_CONNECTION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_REACTION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_POINT_REACTION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_RESULT_GROUP_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_SURFACE_CONNECTION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_SURFACE_MEMBER_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURAL_SURFACE_MEMBER_VARYING_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STRUCTURED_DIMENSION_CALLOUT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STYLE_MODEL_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_STYLED_REPRESENTATION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SUB_CONTRACT_RESOURCE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SUBEDGE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SURFACE_CURVE_SWEPT_AREA_SOLID_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SWEPT_SURFACE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SURFACE_OF_LINEAR_EXTRUSION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SURFACE_OF_REVOLUTION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SURFACE_STYLE_FILL: ClassFill = ClassFill {
    required: Some(3),
    fields: &[
        FieldFill { name: "Side", slot: 1, field_type: FieldType::Text, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "Styles", slot: 2, field_type: FieldType::List(&FieldType::Select, 1, 5), maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_SURFACE_STYLE_SHADING_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "SurfaceColour", slot: 0, field_type: FieldType::Entity, maybe: false, derived_check: true, unset_check: false },
    ],
};
const IFC_SURFACE_STYLE_RENDERING_FILL: ClassFill = ClassFill {
    required: Some(9),
    fields: &[
        FieldFill { name: "Transparency", slot: 1, field_type: FieldType::Real, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "DiffuseColour", slot: 2, field_type: FieldType::Select, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "TransmissionColour", slot: 3, field_type: FieldType::Select, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "DiffuseTransmissionColour", slot: 4, field_type: FieldType::Select, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "ReflectionColour", slot: 5, field_type: FieldType::Select, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "SpecularColour", slot: 6, field_type: FieldType::Select, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "SpecularHighlight", slot: 7, field_type: FieldType::Select, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "ReflectanceMethod", slot: 8, field_type: FieldType::Text, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_SURFACE_STYLE_WITH_TEXTURES_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "Textures", slot: 0, field_type: FieldType::List(&FieldType::Entity, 1, 0), maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_SWEPT_DISK_SOLID_FILL: ClassFill = ClassFill {
    required: Some(5),
    fields: &[
        FieldFill { name: "Directrix", slot: 0, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "Radius", slot: 1, field_type: FieldType::Real, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "InnerRadius", slot: 2, field_type: FieldType::Real, maybe: true, derived_check: false, unset_check: true },
        FieldFill { name: "StartParam", slot: 3, field_type: FieldType::Real, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "EndParam", slot: 4, field_type: FieldType::Real, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_SWITCHING_DEVICE_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_SYSTEM_FURNITURE_ELEMENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_T_SHAPE_PROFILE_DEF_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_TANK_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_TENDON_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_TENDON_ANCHOR_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_TEXT_LITERAL_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_TEXT_LITERAL_WITH_EXTENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_TIME_SERIES_SCHEDULE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_TOPOLOGY_REPRESENTATION_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_TRANSFORMER_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_TRANSPORT_ELEMENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_TRANSPORT_ELEMENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_TRAPEZIUM_PROFILE_DEF_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_TRIMMED_CURVE_FILL: ClassFill = ClassFill {
    required: Some(5),
    fields: &[
        FieldFill { name: "BasisCurve", slot: 0, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "Trim1", slot: 1, field_type: FieldType::List(&FieldType::Select, 1, 2), maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "Trim2", slot: 2, field_type: FieldType::List(&FieldType::Select, 1, 2), maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "SenseAgreement", slot: 3, field_type: FieldType::Text, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "MasterRepresentation", slot: 4, field_type: FieldType::Text, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_TUBE_BUNDLE_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_TWO_DIRECTION_REPEAT_FACTOR_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_U_SHAPE_PROFILE_DEF_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_UNIT_ASSIGNMENT_FILL: ClassFill = ClassFill {
    required: Some(1),
    fields: &[
        FieldFill { name: "Units", slot: 0, field_type: FieldType::List(&FieldType::Select, 1, 0), maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_UNITARY_EQUIPMENT_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_VALVE_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_VECTOR_FILL: ClassFill = ClassFill {
    required: Some(2),
    fields: &[
        FieldFill { name: "Orientation", slot: 0, field_type: FieldType::Entity, maybe: false, derived_check: false, unset_check: false },
        FieldFill { name: "Magnitude", slot: 1, field_type: FieldType::Real, maybe: false, derived_check: false, unset_check: false },
    ],
};
const IFC_VERTEX_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_VERTEX_LOOP_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_VERTEX_POINT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_VIBRATION_ISOLATOR_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_VIRTUAL_ELEMENT_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_WALL_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_WALL_STANDARD_CASE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_WALL_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_WASTE_TERMINAL_TYPE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_WINDOW_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_WINDOW_STYLE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_WORK_CONTROL_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_WORK_PLAN_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_WORK_SCHEDULE_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_Z_SHAPE_PROFILE_DEF_FILL: ClassFill = ClassFill { required: None, fields: &[] };
const IFC_ZONE_FILL: ClassFill = ClassFill { required: None, fields: &[] };

/// The slots of the fields the reader converts, by class.
pub(crate) mod fields {
    use super::{Class, Slot};

    pub(crate) struct IfcCompositeCurve;

    impl IfcCompositeCurve {
        pub const SEGMENTS: Slot = Slot { class: Class::IfcCompositeCurve, index: 0 };
        pub const SELF_INTERSECT: Slot = Slot { class: Class::IfcCompositeCurve, index: 1 };
    }

    pub(crate) struct IfcRoot;

    impl IfcRoot {
        pub const GLOBAL_ID: Slot = Slot { class: Class::IfcRoot, index: 0 };
        pub const OWNER_HISTORY: Slot = Slot { class: Class::IfcRoot, index: 1 };
        pub const NAME: Slot = Slot { class: Class::IfcRoot, index: 2 };
        pub const DESCRIPTION: Slot = Slot { class: Class::IfcRoot, index: 3 };
    }

    pub(crate) struct IfcObject;

    impl IfcObject {
        pub const OBJECT_TYPE: Slot = Slot { class: Class::IfcObject, index: 4 };
    }

    pub(crate) struct IfcProduct;

    impl IfcProduct {
        pub const OBJECT_PLACEMENT: Slot = Slot { class: Class::IfcProduct, index: 5 };
        pub const REPRESENTATION: Slot = Slot { class: Class::IfcProduct, index: 6 };
    }

    pub(crate) struct IfcStyledItem;

    impl IfcStyledItem {
        pub const ITEM: Slot = Slot { class: Class::IfcStyledItem, index: 0 };
        pub const STYLES: Slot = Slot { class: Class::IfcStyledItem, index: 1 };
        pub const NAME: Slot = Slot { class: Class::IfcStyledItem, index: 2 };
    }

    pub(crate) struct IfcProfileDef;

    impl IfcProfileDef {
        pub const PROFILE_TYPE: Slot = Slot { class: Class::IfcProfileDef, index: 0 };
        pub const PROFILE_NAME: Slot = Slot { class: Class::IfcProfileDef, index: 1 };
    }

    pub(crate) struct IfcArbitraryClosedProfileDef;

    impl IfcArbitraryClosedProfileDef {
        pub const OUTER_CURVE: Slot = Slot { class: Class::IfcArbitraryClosedProfileDef, index: 2 };
    }

    pub(crate) struct IfcArbitraryOpenProfileDef;

    impl IfcArbitraryOpenProfileDef {
        pub const CURVE: Slot = Slot { class: Class::IfcArbitraryOpenProfileDef, index: 2 };
    }

    pub(crate) struct IfcParameterizedProfileDef;

    impl IfcParameterizedProfileDef {
        pub const POSITION: Slot = Slot { class: Class::IfcParameterizedProfileDef, index: 2 };
    }

    pub(crate) struct IfcIShapeProfileDef;

    impl IfcIShapeProfileDef {
        pub const OVERALL_WIDTH: Slot = Slot { class: Class::IfcIShapeProfileDef, index: 3 };
        pub const OVERALL_DEPTH: Slot = Slot { class: Class::IfcIShapeProfileDef, index: 4 };
        pub const WEB_THICKNESS: Slot = Slot { class: Class::IfcIShapeProfileDef, index: 5 };
        pub const FLANGE_THICKNESS: Slot = Slot { class: Class::IfcIShapeProfileDef, index: 6 };
        pub const FILLET_RADIUS: Slot = Slot { class: Class::IfcIShapeProfileDef, index: 7 };
    }

    pub(crate) struct IfcPlacement;

    impl IfcPlacement {
        pub const LOCATION: Slot = Slot { class: Class::IfcPlacement, index: 0 };
    }

    pub(crate) struct IfcAxis1Placement;

    impl IfcAxis1Placement {
        pub const AXIS: Slot = Slot { class: Class::IfcAxis1Placement, index: 1 };
    }

    pub(crate) struct IfcAxis2Placement2D;

    impl IfcAxis2Placement2D {
        pub const REF_DIRECTION: Slot = Slot { class: Class::IfcAxis2Placement2D, index: 1 };
    }

    pub(crate) struct IfcAxis2Placement3D;

    impl IfcAxis2Placement3D {
        pub const AXIS: Slot = Slot { class: Class::IfcAxis2Placement3D, index: 1 };
        pub const REF_DIRECTION: Slot = Slot { class: Class::IfcAxis2Placement3D, index: 2 };
    }

    pub(crate) struct IfcBSplineCurve;

    impl IfcBSplineCurve {
        pub const DEGREE: Slot = Slot { class: Class::IfcBSplineCurve, index: 0 };
        pub const CONTROL_POINTS_LIST: Slot = Slot { class: Class::IfcBSplineCurve, index: 1 };
        pub const CURVE_FORM: Slot = Slot { class: Class::IfcBSplineCurve, index: 2 };
        pub const CLOSED_CURVE: Slot = Slot { class: Class::IfcBSplineCurve, index: 3 };
        pub const SELF_INTERSECT: Slot = Slot { class: Class::IfcBSplineCurve, index: 4 };
    }

    pub(crate) struct IfcElement;

    impl IfcElement {
        pub const TAG: Slot = Slot { class: Class::IfcElement, index: 7 };
    }

    pub(crate) struct IfcBooleanResult;

    impl IfcBooleanResult {
        pub const OPERATOR: Slot = Slot { class: Class::IfcBooleanResult, index: 0 };
        pub const FIRST_OPERAND: Slot = Slot { class: Class::IfcBooleanResult, index: 1 };
        pub const SECOND_OPERAND: Slot = Slot { class: Class::IfcBooleanResult, index: 2 };
    }

    pub(crate) struct IfcBoundingBox;

    impl IfcBoundingBox {
        pub const CORNER: Slot = Slot { class: Class::IfcBoundingBox, index: 0 };
        pub const X_DIM: Slot = Slot { class: Class::IfcBoundingBox, index: 1 };
        pub const Y_DIM: Slot = Slot { class: Class::IfcBoundingBox, index: 2 };
        pub const Z_DIM: Slot = Slot { class: Class::IfcBoundingBox, index: 3 };
    }

    pub(crate) struct IfcHalfSpaceSolid;

    impl IfcHalfSpaceSolid {
        pub const BASE_SURFACE: Slot = Slot { class: Class::IfcHalfSpaceSolid, index: 0 };
        pub const AGREEMENT_FLAG: Slot = Slot { class: Class::IfcHalfSpaceSolid, index: 1 };
    }

    pub(crate) struct IfcSpatialStructureElement;

    impl IfcSpatialStructureElement {
        pub const LONG_NAME: Slot = Slot { class: Class::IfcSpatialStructureElement, index: 7 };
        pub const COMPOSITION_TYPE: Slot = Slot { class: Class::IfcSpatialStructureElement, index: 8 };
    }

    pub(crate) struct IfcBuilding;

    impl IfcBuilding {
        pub const ELEVATION_OF_REF_HEIGHT: Slot = Slot { class: Class::IfcBuilding, index: 9 };
        pub const ELEVATION_OF_TERRAIN: Slot = Slot { class: Class::IfcBuilding, index: 10 };
        pub const BUILDING_ADDRESS: Slot = Slot { class: Class::IfcBuilding, index: 11 };
    }

    pub(crate) struct IfcCartesianPoint;

    impl IfcCartesianPoint {
        pub const COORDINATES: Slot = Slot { class: Class::IfcCartesianPoint, index: 0 };
    }

    pub(crate) struct IfcCartesianTransformationOperator;

    impl IfcCartesianTransformationOperator {
        pub const AXIS1: Slot = Slot { class: Class::IfcCartesianTransformationOperator, index: 0 };
        pub const AXIS2: Slot = Slot { class: Class::IfcCartesianTransformationOperator, index: 1 };
        pub const LOCAL_ORIGIN: Slot = Slot { class: Class::IfcCartesianTransformationOperator, index: 2 };
        pub const SCALE: Slot = Slot { class: Class::IfcCartesianTransformationOperator, index: 3 };
    }

    pub(crate) struct IfcCartesianTransformationOperator3D;

    impl IfcCartesianTransformationOperator3D {
        pub const AXIS3: Slot = Slot { class: Class::IfcCartesianTransformationOperator3D, index: 4 };
    }

    pub(crate) struct IfcCartesianTransformationOperator3DnonUniform;

    impl IfcCartesianTransformationOperator3DnonUniform {
        pub const SCALE2: Slot = Slot { class: Class::IfcCartesianTransformationOperator3DnonUniform, index: 5 };
        pub const SCALE3: Slot = Slot { class: Class::IfcCartesianTransformationOperator3DnonUniform, index: 6 };
    }

    pub(crate) struct IfcConic;

    impl IfcConic {
        pub const POSITION: Slot = Slot { class: Class::IfcConic, index: 0 };
    }

    pub(crate) struct IfcCircle;

    impl IfcCircle {
        pub const RADIUS: Slot = Slot { class: Class::IfcCircle, index: 1 };
    }

    pub(crate) struct IfcCircleProfileDef;

    impl IfcCircleProfileDef {
        pub const RADIUS: Slot = Slot { class: Class::IfcCircleProfileDef, index: 3 };
    }

    pub(crate) struct IfcCircleHollowProfileDef;

    impl IfcCircleHollowProfileDef {
        pub const WALL_THICKNESS: Slot = Slot { class: Class::IfcCircleHollowProfileDef, index: 4 };
    }

    pub(crate) struct IfcConnectedFaceSet;

    impl IfcConnectedFaceSet {
        pub const CFS_FACES: Slot = Slot { class: Class::IfcConnectedFaceSet, index: 0 };
    }

    pub(crate) struct IfcColourSpecification;

    impl IfcColourSpecification {
        pub const NAME: Slot = Slot { class: Class::IfcColourSpecification, index: 0 };
    }

    pub(crate) struct IfcColourRgb;

    impl IfcColourRgb {
        pub const RED: Slot = Slot { class: Class::IfcColourRgb, index: 1 };
        pub const GREEN: Slot = Slot { class: Class::IfcColourRgb, index: 2 };
        pub const BLUE: Slot = Slot { class: Class::IfcColourRgb, index: 3 };
    }

    pub(crate) struct IfcProperty;

    impl IfcProperty {
        pub const NAME: Slot = Slot { class: Class::IfcProperty, index: 0 };
        pub const DESCRIPTION: Slot = Slot { class: Class::IfcProperty, index: 1 };
    }

    pub(crate) struct IfcComplexProperty;

    impl IfcComplexProperty {
        pub const USAGE_NAME: Slot = Slot { class: Class::IfcComplexProperty, index: 2 };
        pub const HAS_PROPERTIES: Slot = Slot { class: Class::IfcComplexProperty, index: 3 };
    }

    pub(crate) struct IfcCompositeCurveSegment;

    impl IfcCompositeCurveSegment {
        pub const TRANSITION: Slot = Slot { class: Class::IfcCompositeCurveSegment, index: 0 };
        pub const SAME_SENSE: Slot = Slot { class: Class::IfcCompositeCurveSegment, index: 1 };
        pub const PARENT_CURVE: Slot = Slot { class: Class::IfcCompositeCurveSegment, index: 2 };
    }

    pub(crate) struct IfcNamedUnit;

    impl IfcNamedUnit {
        pub const DIMENSIONS: Slot = Slot { class: Class::IfcNamedUnit, index: 0 };
        pub const UNIT_TYPE: Slot = Slot { class: Class::IfcNamedUnit, index: 1 };
    }

    pub(crate) struct IfcConversionBasedUnit;

    impl IfcConversionBasedUnit {
        pub const NAME: Slot = Slot { class: Class::IfcConversionBasedUnit, index: 2 };
        pub const CONVERSION_FACTOR: Slot = Slot { class: Class::IfcConversionBasedUnit, index: 3 };
    }

    pub(crate) struct IfcPresentationStyle;

    impl IfcPresentationStyle {
        pub const NAME: Slot = Slot { class: Class::IfcPresentationStyle, index: 0 };
    }

    pub(crate) struct IfcDirection;

    impl IfcDirection {
        pub const DIRECTION_RATIOS: Slot = Slot { class: Class::IfcDirection, index: 0 };
    }

    pub(crate) struct IfcDoor;

    impl IfcDoor {
        pub const OVERALL_HEIGHT: Slot = Slot { class: Class::IfcDoor, index: 8 };
        pub const OVERALL_WIDTH: Slot = Slot { class: Class::IfcDoor, index: 9 };
    }

    pub(crate) struct IfcElementQuantity;

    impl IfcElementQuantity {
        pub const METHOD_OF_MEASUREMENT: Slot = Slot { class: Class::IfcElementQuantity, index: 4 };
        pub const QUANTITIES: Slot = Slot { class: Class::IfcElementQuantity, index: 5 };
    }

    pub(crate) struct IfcElementarySurface;

    impl IfcElementarySurface {
        pub const POSITION: Slot = Slot { class: Class::IfcElementarySurface, index: 0 };
    }

    pub(crate) struct IfcEllipse;

    impl IfcEllipse {
        pub const SEMI_AXIS1: Slot = Slot { class: Class::IfcEllipse, index: 1 };
        pub const SEMI_AXIS2: Slot = Slot { class: Class::IfcEllipse, index: 2 };
    }

    pub(crate) struct IfcSweptAreaSolid;

    impl IfcSweptAreaSolid {
        pub const SWEPT_AREA: Slot = Slot { class: Class::IfcSweptAreaSolid, index: 0 };
        pub const POSITION: Slot = Slot { class: Class::IfcSweptAreaSolid, index: 1 };
    }

    pub(crate) struct IfcExtrudedAreaSolid;

    impl IfcExtrudedAreaSolid {
        pub const EXTRUDED_DIRECTION: Slot = Slot { class: Class::IfcExtrudedAreaSolid, index: 2 };
        pub const DEPTH: Slot = Slot { class: Class::IfcExtrudedAreaSolid, index: 3 };
    }

    pub(crate) struct IfcFace;

    impl IfcFace {
        pub const BOUNDS: Slot = Slot { class: Class::IfcFace, index: 0 };
    }

    pub(crate) struct IfcFaceBasedSurfaceModel;

    impl IfcFaceBasedSurfaceModel {
        pub const FBSM_FACES: Slot = Slot { class: Class::IfcFaceBasedSurfaceModel, index: 0 };
    }

    pub(crate) struct IfcFaceBound;

    impl IfcFaceBound {
        pub const BOUND: Slot = Slot { class: Class::IfcFaceBound, index: 0 };
        pub const ORIENTATION: Slot = Slot { class: Class::IfcFaceBound, index: 1 };
    }

    pub(crate) struct IfcManifoldSolidBrep;

    impl IfcManifoldSolidBrep {
        pub const OUTER: Slot = Slot { class: Class::IfcManifoldSolidBrep, index: 0 };
    }

    pub(crate) struct IfcRepresentationContext;

    impl IfcRepresentationContext {
        pub const CONTEXT_IDENTIFIER: Slot = Slot { class: Class::IfcRepresentationContext, index: 0 };
        pub const CONTEXT_TYPE: Slot = Slot { class: Class::IfcRepresentationContext, index: 1 };
    }

    pub(crate) struct IfcGeometricRepresentationContext;

    impl IfcGeometricRepresentationContext {
        pub const COORDINATE_SPACE_DIMENSION: Slot = Slot { class: Class::IfcGeometricRepresentationContext, index: 2 };
        pub const PRECISION: Slot = Slot { class: Class::IfcGeometricRepresentationContext, index: 3 };
        pub const WORLD_COORDINATE_SYSTEM: Slot = Slot { class: Class::IfcGeometricRepresentationContext, index: 4 };
        pub const TRUE_NORTH: Slot = Slot { class: Class::IfcGeometricRepresentationContext, index: 5 };
    }

    pub(crate) struct IfcLine;

    impl IfcLine {
        pub const PNT: Slot = Slot { class: Class::IfcLine, index: 0 };
        pub const DIR: Slot = Slot { class: Class::IfcLine, index: 1 };
    }

    pub(crate) struct IfcLocalPlacement;

    impl IfcLocalPlacement {
        pub const PLACEMENT_REL_TO: Slot = Slot { class: Class::IfcLocalPlacement, index: 0 };
        pub const RELATIVE_PLACEMENT: Slot = Slot { class: Class::IfcLocalPlacement, index: 1 };
    }

    pub(crate) struct IfcMappedItem;

    impl IfcMappedItem {
        pub const MAPPING_SOURCE: Slot = Slot { class: Class::IfcMappedItem, index: 0 };
        pub const MAPPING_TARGET: Slot = Slot { class: Class::IfcMappedItem, index: 1 };
    }

    pub(crate) struct IfcProductRepresentation;

    impl IfcProductRepresentation {
        pub const NAME: Slot = Slot { class: Class::IfcProductRepresentation, index: 0 };
        pub const DESCRIPTION: Slot = Slot { class: Class::IfcProductRepresentation, index: 1 };
        pub const REPRESENTATIONS: Slot = Slot { class: Class::IfcProductRepresentation, index: 2 };
    }

    pub(crate) struct IfcMeasureWithUnit;

    impl IfcMeasureWithUnit {
        pub const VALUE_COMPONENT: Slot = Slot { class: Class::IfcMeasureWithUnit, index: 0 };
        pub const UNIT_COMPONENT: Slot = Slot { class: Class::IfcMeasureWithUnit, index: 1 };
    }

    pub(crate) struct IfcPolyLoop;

    impl IfcPolyLoop {
        pub const POLYGON: Slot = Slot { class: Class::IfcPolyLoop, index: 0 };
    }

    pub(crate) struct IfcPolygonalBoundedHalfSpace;

    impl IfcPolygonalBoundedHalfSpace {
        pub const POSITION: Slot = Slot { class: Class::IfcPolygonalBoundedHalfSpace, index: 2 };
        pub const POLYGONAL_BOUNDARY: Slot = Slot { class: Class::IfcPolygonalBoundedHalfSpace, index: 3 };
    }

    pub(crate) struct IfcPolyline;

    impl IfcPolyline {
        pub const POINTS: Slot = Slot { class: Class::IfcPolyline, index: 0 };
    }

    pub(crate) struct IfcPresentationStyleAssignment;

    impl IfcPresentationStyleAssignment {
        pub const STYLES: Slot = Slot { class: Class::IfcPresentationStyleAssignment, index: 0 };
    }

    pub(crate) struct IfcProject;

    impl IfcProject {
        pub const LONG_NAME: Slot = Slot { class: Class::IfcProject, index: 5 };
        pub const PHASE: Slot = Slot { class: Class::IfcProject, index: 6 };
        pub const REPRESENTATION_CONTEXTS: Slot = Slot { class: Class::IfcProject, index: 7 };
        pub const UNITS_IN_CONTEXT: Slot = Slot { class: Class::IfcProject, index: 8 };
    }

    pub(crate) struct IfcPropertyListValue;

    impl IfcPropertyListValue {
        pub const LIST_VALUES: Slot = Slot { class: Class::IfcPropertyListValue, index: 2 };
        pub const UNIT: Slot = Slot { class: Class::IfcPropertyListValue, index: 3 };
    }

    pub(crate) struct IfcPropertySet;

    impl IfcPropertySet {
        pub const HAS_PROPERTIES: Slot = Slot { class: Class::IfcPropertySet, index: 4 };
    }

    pub(crate) struct IfcPropertySingleValue;

    impl IfcPropertySingleValue {
        pub const NOMINAL_VALUE: Slot = Slot { class: Class::IfcPropertySingleValue, index: 2 };
        pub const UNIT: Slot = Slot { class: Class::IfcPropertySingleValue, index: 3 };
    }

    pub(crate) struct IfcRectangleProfileDef;

    impl IfcRectangleProfileDef {
        pub const X_DIM: Slot = Slot { class: Class::IfcRectangleProfileDef, index: 3 };
        pub const Y_DIM: Slot = Slot { class: Class::IfcRectangleProfileDef, index: 4 };
    }

    pub(crate) struct IfcRelDecomposes;

    impl IfcRelDecomposes {
        pub const RELATING_OBJECT: Slot = Slot { class: Class::IfcRelDecomposes, index: 4 };
        pub const RELATED_OBJECTS: Slot = Slot { class: Class::IfcRelDecomposes, index: 5 };
    }

    pub(crate) struct IfcRelContainedInSpatialStructure;

    impl IfcRelContainedInSpatialStructure {
        pub const RELATED_ELEMENTS: Slot = Slot { class: Class::IfcRelContainedInSpatialStructure, index: 4 };
        pub const RELATING_STRUCTURE: Slot = Slot { class: Class::IfcRelContainedInSpatialStructure, index: 5 };
    }

    pub(crate) struct IfcRelDefines;

    impl IfcRelDefines {
        pub const RELATED_OBJECTS: Slot = Slot { class: Class::IfcRelDefines, index: 4 };
    }

    pub(crate) struct IfcRelDefinesByProperties;

    impl IfcRelDefinesByProperties {
        pub const RELATING_PROPERTY_DEFINITION: Slot = Slot { class: Class::IfcRelDefinesByProperties, index: 5 };
    }

    pub(crate) struct IfcRelFillsElement;

    impl IfcRelFillsElement {
        pub const RELATING_OPENING_ELEMENT: Slot = Slot { class: Class::IfcRelFillsElement, index: 4 };
        pub const RELATED_BUILDING_ELEMENT: Slot = Slot { class: Class::IfcRelFillsElement, index: 5 };
    }

    pub(crate) struct IfcRelVoidsElement;

    impl IfcRelVoidsElement {
        pub const RELATING_BUILDING_ELEMENT: Slot = Slot { class: Class::IfcRelVoidsElement, index: 4 };
        pub const RELATED_OPENING_ELEMENT: Slot = Slot { class: Class::IfcRelVoidsElement, index: 5 };
    }

    pub(crate) struct IfcRepresentation;

    impl IfcRepresentation {
        pub const CONTEXT_OF_ITEMS: Slot = Slot { class: Class::IfcRepresentation, index: 0 };
        pub const REPRESENTATION_IDENTIFIER: Slot = Slot { class: Class::IfcRepresentation, index: 1 };
        pub const REPRESENTATION_TYPE: Slot = Slot { class: Class::IfcRepresentation, index: 2 };
        pub const ITEMS: Slot = Slot { class: Class::IfcRepresentation, index: 3 };
    }

    pub(crate) struct IfcRepresentationMap;

    impl IfcRepresentationMap {
        pub const MAPPING_ORIGIN: Slot = Slot { class: Class::IfcRepresentationMap, index: 0 };
        pub const MAPPED_REPRESENTATION: Slot = Slot { class: Class::IfcRepresentationMap, index: 1 };
    }

    pub(crate) struct IfcRevolvedAreaSolid;

    impl IfcRevolvedAreaSolid {
        pub const AXIS: Slot = Slot { class: Class::IfcRevolvedAreaSolid, index: 2 };
        pub const ANGLE: Slot = Slot { class: Class::IfcRevolvedAreaSolid, index: 3 };
    }

    pub(crate) struct IfcSIUnit;

    impl IfcSIUnit {
        pub const PREFIX: Slot = Slot { class: Class::IfcSIUnit, index: 2 };
        pub const NAME: Slot = Slot { class: Class::IfcSIUnit, index: 3 };
    }

    pub(crate) struct IfcShellBasedSurfaceModel;

    impl IfcShellBasedSurfaceModel {
        pub const SBSM_BOUNDARY: Slot = Slot { class: Class::IfcShellBasedSurfaceModel, index: 0 };
    }

    pub(crate) struct IfcSite;

    impl IfcSite {
        pub const REF_LATITUDE: Slot = Slot { class: Class::IfcSite, index: 9 };
        pub const REF_LONGITUDE: Slot = Slot { class: Class::IfcSite, index: 10 };
        pub const REF_ELEVATION: Slot = Slot { class: Class::IfcSite, index: 11 };
        pub const LAND_TITLE_NUMBER: Slot = Slot { class: Class::IfcSite, index: 12 };
        pub const SITE_ADDRESS: Slot = Slot { class: Class::IfcSite, index: 13 };
    }

    pub(crate) struct IfcSpace;

    impl IfcSpace {
        pub const INTERIOR_OR_EXTERIOR_SPACE: Slot = Slot { class: Class::IfcSpace, index: 9 };
        pub const ELEVATION_WITH_FLOORING: Slot = Slot { class: Class::IfcSpace, index: 10 };
    }

    pub(crate) struct IfcSurfaceStyle;

    impl IfcSurfaceStyle {
        pub const SIDE: Slot = Slot { class: Class::IfcSurfaceStyle, index: 1 };
        pub const STYLES: Slot = Slot { class: Class::IfcSurfaceStyle, index: 2 };
    }

    pub(crate) struct IfcSurfaceStyleShading;

    impl IfcSurfaceStyleShading {
        pub const SURFACE_COLOUR: Slot = Slot { class: Class::IfcSurfaceStyleShading, index: 0 };
    }

    pub(crate) struct IfcSurfaceStyleRendering;

    impl IfcSurfaceStyleRendering {
        pub const TRANSPARENCY: Slot = Slot { class: Class::IfcSurfaceStyleRendering, index: 1 };
        pub const DIFFUSE_COLOUR: Slot = Slot { class: Class::IfcSurfaceStyleRendering, index: 2 };
        pub const TRANSMISSION_COLOUR: Slot = Slot { class: Class::IfcSurfaceStyleRendering, index: 3 };
        pub const DIFFUSE_TRANSMISSION_COLOUR: Slot = Slot { class: Class::IfcSurfaceStyleRendering, index: 4 };
        pub const REFLECTION_COLOUR: Slot = Slot { class: Class::IfcSurfaceStyleRendering, index: 5 };
        pub const SPECULAR_COLOUR: Slot = Slot { class: Class::IfcSurfaceStyleRendering, index: 6 };
        pub const SPECULAR_HIGHLIGHT: Slot = Slot { class: Class::IfcSurfaceStyleRendering, index: 7 };
        pub const REFLECTANCE_METHOD: Slot = Slot { class: Class::IfcSurfaceStyleRendering, index: 8 };
    }

    pub(crate) struct IfcSurfaceStyleWithTextures;

    impl IfcSurfaceStyleWithTextures {
        pub const TEXTURES: Slot = Slot { class: Class::IfcSurfaceStyleWithTextures, index: 0 };
    }

    pub(crate) struct IfcSweptDiskSolid;

    impl IfcSweptDiskSolid {
        pub const DIRECTRIX: Slot = Slot { class: Class::IfcSweptDiskSolid, index: 0 };
        pub const RADIUS: Slot = Slot { class: Class::IfcSweptDiskSolid, index: 1 };
        pub const INNER_RADIUS: Slot = Slot { class: Class::IfcSweptDiskSolid, index: 2 };
        pub const START_PARAM: Slot = Slot { class: Class::IfcSweptDiskSolid, index: 3 };
        pub const END_PARAM: Slot = Slot { class: Class::IfcSweptDiskSolid, index: 4 };
    }

    pub(crate) struct IfcTrimmedCurve;

    impl IfcTrimmedCurve {
        pub const BASIS_CURVE: Slot = Slot { class: Class::IfcTrimmedCurve, index: 0 };
        pub const TRIM1: Slot = Slot { class: Class::IfcTrimmedCurve, index: 1 };
        pub const TRIM2: Slot = Slot { class: Class::IfcTrimmedCurve, index: 2 };
        pub const SENSE_AGREEMENT: Slot = Slot { class: Class::IfcTrimmedCurve, index: 3 };
        pub const MASTER_REPRESENTATION: Slot = Slot { class: Class::IfcTrimmedCurve, index: 4 };
    }

    pub(crate) struct IfcUnitAssignment;

    impl IfcUnitAssignment {
        pub const UNITS: Slot = Slot { class: Class::IfcUnitAssignment, index: 0 };
    }

    pub(crate) struct IfcVector;

    impl IfcVector {
        pub const ORIENTATION: Slot = Slot { class: Class::IfcVector, index: 0 };
        pub const MAGNITUDE: Slot = Slot { class: Class::IfcVector, index: 1 };
    }
}

/// The converter `ConversionSchema` has for a name in the schema.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Converter {
    /// Listed with no converter (types that aren't entities).
    Missing,
    /// `ObjectHelper<NotImplemented, 0>`: an object of no class.
    NotImplemented,
    Class(Class),
}

/// The schema's names (lower case) and converters, sorted by name.
pub(crate) const SCHEMA: &[(&str, Converter)] = &[
    ("ifc2dcompositecurve", Converter::Class(Class::Ifc2DCompositeCurve)),
    ("ifcabsorbeddosemeasure", Converter::Missing),
    ("ifcaccelerationmeasure", Converter::Missing),
    ("ifcactionrequest", Converter::Class(Class::IfcActionRequest)),
    ("ifcactionsourcetypeenum", Converter::Missing),
    ("ifcactiontypeenum", Converter::Missing),
    ("ifcactor", Converter::Class(Class::IfcActor)),
    ("ifcactorrole", Converter::NotImplemented),
    ("ifcactorselect", Converter::Missing),
    ("ifcactuatortype", Converter::Class(Class::IfcActuatorType)),
    ("ifcactuatortypeenum", Converter::Missing),
    ("ifcaddress", Converter::NotImplemented),
    ("ifcaddresstypeenum", Converter::Missing),
    ("ifcaheadorbehind", Converter::Missing),
    ("ifcairterminalboxtype", Converter::Class(Class::IfcAirTerminalBoxType)),
    ("ifcairterminalboxtypeenum", Converter::Missing),
    ("ifcairterminaltype", Converter::Class(Class::IfcAirTerminalType)),
    ("ifcairterminaltypeenum", Converter::Missing),
    ("ifcairtoairheatrecoverytype", Converter::Class(Class::IfcAirToAirHeatRecoveryType)),
    ("ifcairtoairheatrecoverytypeenum", Converter::Missing),
    ("ifcalarmtype", Converter::Class(Class::IfcAlarmType)),
    ("ifcalarmtypeenum", Converter::Missing),
    ("ifcamountofsubstancemeasure", Converter::Missing),
    ("ifcanalysismodeltypeenum", Converter::Missing),
    ("ifcanalysistheorytypeenum", Converter::Missing),
    ("ifcangulardimension", Converter::Class(Class::IfcAngularDimension)),
    ("ifcangularvelocitymeasure", Converter::Missing),
    ("ifcannotation", Converter::Class(Class::IfcAnnotation)),
    ("ifcannotationcurveoccurrence", Converter::Class(Class::IfcAnnotationCurveOccurrence)),
    ("ifcannotationfillarea", Converter::Class(Class::IfcAnnotationFillArea)),
    ("ifcannotationfillareaoccurrence", Converter::Class(Class::IfcAnnotationFillAreaOccurrence)),
    ("ifcannotationoccurrence", Converter::Class(Class::IfcAnnotationOccurrence)),
    ("ifcannotationsurface", Converter::Class(Class::IfcAnnotationSurface)),
    ("ifcannotationsurfaceoccurrence", Converter::Class(Class::IfcAnnotationSurfaceOccurrence)),
    ("ifcannotationsymboloccurrence", Converter::Class(Class::IfcAnnotationSymbolOccurrence)),
    ("ifcannotationtextoccurrence", Converter::Class(Class::IfcAnnotationTextOccurrence)),
    ("ifcapplication", Converter::NotImplemented),
    ("ifcappliedvalue", Converter::NotImplemented),
    ("ifcappliedvaluerelationship", Converter::NotImplemented),
    ("ifcappliedvalueselect", Converter::Missing),
    ("ifcapproval", Converter::NotImplemented),
    ("ifcapprovalactorrelationship", Converter::NotImplemented),
    ("ifcapprovalpropertyrelationship", Converter::NotImplemented),
    ("ifcapprovalrelationship", Converter::NotImplemented),
    ("ifcarbitraryclosedprofiledef", Converter::Class(Class::IfcArbitraryClosedProfileDef)),
    ("ifcarbitraryopenprofiledef", Converter::Class(Class::IfcArbitraryOpenProfileDef)),
    ("ifcarbitraryprofiledefwithvoids", Converter::Class(Class::IfcArbitraryProfileDefWithVoids)),
    ("ifcareameasure", Converter::Missing),
    ("ifcarithmeticoperatorenum", Converter::Missing),
    ("ifcassemblyplaceenum", Converter::Missing),
    ("ifcasset", Converter::Class(Class::IfcAsset)),
    ("ifcasymmetricishapeprofiledef", Converter::Class(Class::IfcAsymmetricIShapeProfileDef)),
    ("ifcaxis1placement", Converter::Class(Class::IfcAxis1Placement)),
    ("ifcaxis2placement", Converter::Missing),
    ("ifcaxis2placement2d", Converter::Class(Class::IfcAxis2Placement2D)),
    ("ifcaxis2placement3d", Converter::Class(Class::IfcAxis2Placement3D)),
    ("ifcbeam", Converter::Class(Class::IfcBeam)),
    ("ifcbeamtype", Converter::Class(Class::IfcBeamType)),
    ("ifcbeamtypeenum", Converter::Missing),
    ("ifcbenchmarkenum", Converter::Missing),
    ("ifcbeziercurve", Converter::Class(Class::IfcBezierCurve)),
    ("ifcblobtexture", Converter::NotImplemented),
    ("ifcblock", Converter::Class(Class::IfcBlock)),
    ("ifcboilertype", Converter::Class(Class::IfcBoilerType)),
    ("ifcboilertypeenum", Converter::Missing),
    ("ifcboolean", Converter::Missing),
    ("ifcbooleanclippingresult", Converter::Class(Class::IfcBooleanClippingResult)),
    ("ifcbooleanoperand", Converter::Missing),
    ("ifcbooleanoperator", Converter::Missing),
    ("ifcbooleanresult", Converter::Class(Class::IfcBooleanResult)),
    ("ifcboundarycondition", Converter::NotImplemented),
    ("ifcboundaryedgecondition", Converter::NotImplemented),
    ("ifcboundaryfacecondition", Converter::NotImplemented),
    ("ifcboundarynodecondition", Converter::NotImplemented),
    ("ifcboundarynodeconditionwarping", Converter::NotImplemented),
    ("ifcboundedcurve", Converter::Class(Class::IfcBoundedCurve)),
    ("ifcboundedsurface", Converter::Class(Class::IfcBoundedSurface)),
    ("ifcboundingbox", Converter::Class(Class::IfcBoundingBox)),
    ("ifcboxalignment", Converter::Missing),
    ("ifcboxedhalfspace", Converter::Class(Class::IfcBoxedHalfSpace)),
    ("ifcbsplinecurve", Converter::Class(Class::IfcBSplineCurve)),
    ("ifcbsplinecurveform", Converter::Missing),
    ("ifcbuilding", Converter::Class(Class::IfcBuilding)),
    ("ifcbuildingelement", Converter::Class(Class::IfcBuildingElement)),
    ("ifcbuildingelementcomponent", Converter::Class(Class::IfcBuildingElementComponent)),
    ("ifcbuildingelementpart", Converter::Class(Class::IfcBuildingElementPart)),
    ("ifcbuildingelementproxy", Converter::Class(Class::IfcBuildingElementProxy)),
    ("ifcbuildingelementproxytype", Converter::Class(Class::IfcBuildingElementProxyType)),
    ("ifcbuildingelementproxytypeenum", Converter::Missing),
    ("ifcbuildingelementtype", Converter::Class(Class::IfcBuildingElementType)),
    ("ifcbuildingstorey", Converter::Class(Class::IfcBuildingStorey)),
    ("ifccablecarrierfittingtype", Converter::Class(Class::IfcCableCarrierFittingType)),
    ("ifccablecarrierfittingtypeenum", Converter::Missing),
    ("ifccablecarriersegmenttype", Converter::Class(Class::IfcCableCarrierSegmentType)),
    ("ifccablecarriersegmenttypeenum", Converter::Missing),
    ("ifccablesegmenttype", Converter::Class(Class::IfcCableSegmentType)),
    ("ifccablesegmenttypeenum", Converter::Missing),
    ("ifccalendardate", Converter::NotImplemented),
    ("ifccartesianpoint", Converter::Class(Class::IfcCartesianPoint)),
    ("ifccartesiantransformationoperator", Converter::Class(Class::IfcCartesianTransformationOperator)),
    ("ifccartesiantransformationoperator2d", Converter::Class(Class::IfcCartesianTransformationOperator2D)),
    ("ifccartesiantransformationoperator2dnonuniform", Converter::Class(Class::IfcCartesianTransformationOperator2DnonUniform)),
    ("ifccartesiantransformationoperator3d", Converter::Class(Class::IfcCartesianTransformationOperator3D)),
    ("ifccartesiantransformationoperator3dnonuniform", Converter::Class(Class::IfcCartesianTransformationOperator3DnonUniform)),
    ("ifccenterlineprofiledef", Converter::Class(Class::IfcCenterLineProfileDef)),
    ("ifcchamferedgefeature", Converter::Class(Class::IfcChamferEdgeFeature)),
    ("ifcchangeactionenum", Converter::Missing),
    ("ifccharacterstyleselect", Converter::Missing),
    ("ifcchillertype", Converter::Class(Class::IfcChillerType)),
    ("ifcchillertypeenum", Converter::Missing),
    ("ifccircle", Converter::Class(Class::IfcCircle)),
    ("ifccirclehollowprofiledef", Converter::Class(Class::IfcCircleHollowProfileDef)),
    ("ifccircleprofiledef", Converter::Class(Class::IfcCircleProfileDef)),
    ("ifcclassification", Converter::NotImplemented),
    ("ifcclassificationitem", Converter::NotImplemented),
    ("ifcclassificationitemrelationship", Converter::NotImplemented),
    ("ifcclassificationnotation", Converter::NotImplemented),
    ("ifcclassificationnotationfacet", Converter::NotImplemented),
    ("ifcclassificationnotationselect", Converter::Missing),
    ("ifcclassificationreference", Converter::NotImplemented),
    ("ifcclosedshell", Converter::Class(Class::IfcClosedShell)),
    ("ifccoiltype", Converter::Class(Class::IfcCoilType)),
    ("ifccoiltypeenum", Converter::Missing),
    ("ifccolour", Converter::Missing),
    ("ifccolourorfactor", Converter::Missing),
    ("ifccolourrgb", Converter::Class(Class::IfcColourRgb)),
    ("ifccolourspecification", Converter::Class(Class::IfcColourSpecification)),
    ("ifccolumn", Converter::Class(Class::IfcColumn)),
    ("ifccolumntype", Converter::Class(Class::IfcColumnType)),
    ("ifccolumntypeenum", Converter::Missing),
    ("ifccomplexnumber", Converter::Missing),
    ("ifccomplexproperty", Converter::Class(Class::IfcComplexProperty)),
    ("ifccompositecurve", Converter::Class(Class::IfcCompositeCurve)),
    ("ifccompositecurvesegment", Converter::Class(Class::IfcCompositeCurveSegment)),
    ("ifccompositeprofiledef", Converter::Class(Class::IfcCompositeProfileDef)),
    ("ifccompoundplaneanglemeasure", Converter::Missing),
    ("ifccompressortype", Converter::Class(Class::IfcCompressorType)),
    ("ifccompressortypeenum", Converter::Missing),
    ("ifccondensertype", Converter::Class(Class::IfcCondenserType)),
    ("ifccondensertypeenum", Converter::Missing),
    ("ifccondition", Converter::Class(Class::IfcCondition)),
    ("ifcconditioncriterion", Converter::Class(Class::IfcConditionCriterion)),
    ("ifcconditioncriterionselect", Converter::Missing),
    ("ifcconic", Converter::Class(Class::IfcConic)),
    ("ifcconnectedfaceset", Converter::Class(Class::IfcConnectedFaceSet)),
    ("ifcconnectioncurvegeometry", Converter::NotImplemented),
    ("ifcconnectiongeometry", Converter::NotImplemented),
    ("ifcconnectionpointeccentricity", Converter::NotImplemented),
    ("ifcconnectionpointgeometry", Converter::NotImplemented),
    ("ifcconnectionportgeometry", Converter::NotImplemented),
    ("ifcconnectionsurfacegeometry", Converter::NotImplemented),
    ("ifcconnectiontypeenum", Converter::Missing),
    ("ifcconstraint", Converter::NotImplemented),
    ("ifcconstraintaggregationrelationship", Converter::NotImplemented),
    ("ifcconstraintclassificationrelationship", Converter::NotImplemented),
    ("ifcconstraintenum", Converter::Missing),
    ("ifcconstraintrelationship", Converter::NotImplemented),
    ("ifcconstructionequipmentresource", Converter::Class(Class::IfcConstructionEquipmentResource)),
    ("ifcconstructionmaterialresource", Converter::Class(Class::IfcConstructionMaterialResource)),
    ("ifcconstructionproductresource", Converter::Class(Class::IfcConstructionProductResource)),
    ("ifcconstructionresource", Converter::Class(Class::IfcConstructionResource)),
    ("ifccontextdependentmeasure", Converter::Missing),
    ("ifccontextdependentunit", Converter::Class(Class::IfcContextDependentUnit)),
    ("ifccontrol", Converter::Class(Class::IfcControl)),
    ("ifccontrollertype", Converter::Class(Class::IfcControllerType)),
    ("ifccontrollertypeenum", Converter::Missing),
    ("ifcconversionbasedunit", Converter::Class(Class::IfcConversionBasedUnit)),
    ("ifccooledbeamtype", Converter::Class(Class::IfcCooledBeamType)),
    ("ifccooledbeamtypeenum", Converter::Missing),
    ("ifccoolingtowertype", Converter::Class(Class::IfcCoolingTowerType)),
    ("ifccoolingtowertypeenum", Converter::Missing),
    ("ifccoordinateduniversaltimeoffset", Converter::NotImplemented),
    ("ifccostitem", Converter::Class(Class::IfcCostItem)),
    ("ifccostschedule", Converter::Class(Class::IfcCostSchedule)),
    ("ifccostscheduletypeenum", Converter::Missing),
    ("ifccostvalue", Converter::NotImplemented),
    ("ifccountmeasure", Converter::Missing),
    ("ifccovering", Converter::Class(Class::IfcCovering)),
    ("ifccoveringtype", Converter::Class(Class::IfcCoveringType)),
    ("ifccoveringtypeenum", Converter::Missing),
    ("ifccranerailashapeprofiledef", Converter::Class(Class::IfcCraneRailAShapeProfileDef)),
    ("ifccranerailfshapeprofiledef", Converter::Class(Class::IfcCraneRailFShapeProfileDef)),
    ("ifccrewresource", Converter::Class(Class::IfcCrewResource)),
    ("ifccsgprimitive3d", Converter::Class(Class::IfcCsgPrimitive3D)),
    ("ifccsgselect", Converter::Missing),
    ("ifccsgsolid", Converter::Class(Class::IfcCsgSolid)),
    ("ifccshapeprofiledef", Converter::Class(Class::IfcCShapeProfileDef)),
    ("ifccurrencyenum", Converter::Missing),
    ("ifccurrencyrelationship", Converter::NotImplemented),
    ("ifccurtainwall", Converter::Class(Class::IfcCurtainWall)),
    ("ifccurtainwalltype", Converter::Class(Class::IfcCurtainWallType)),
    ("ifccurtainwalltypeenum", Converter::Missing),
    ("ifccurvaturemeasure", Converter::Missing),
    ("ifccurve", Converter::Class(Class::IfcCurve)),
    ("ifccurveboundedplane", Converter::Class(Class::IfcCurveBoundedPlane)),
    ("ifccurvefontorscaledcurvefontselect", Converter::Missing),
    ("ifccurveoredgecurve", Converter::Missing),
    ("ifccurvestyle", Converter::NotImplemented),
    ("ifccurvestylefont", Converter::NotImplemented),
    ("ifccurvestylefontandscaling", Converter::NotImplemented),
    ("ifccurvestylefontpattern", Converter::NotImplemented),
    ("ifccurvestylefontselect", Converter::Missing),
    ("ifcdampertype", Converter::Class(Class::IfcDamperType)),
    ("ifcdampertypeenum", Converter::Missing),
    ("ifcdataoriginenum", Converter::Missing),
    ("ifcdateandtime", Converter::NotImplemented),
    ("ifcdatetimeselect", Converter::Missing),
    ("ifcdayinmonthnumber", Converter::Missing),
    ("ifcdaylightsavinghour", Converter::Missing),
    ("ifcdefinedsymbol", Converter::Class(Class::IfcDefinedSymbol)),
    ("ifcdefinedsymbolselect", Converter::Missing),
    ("ifcderivedmeasurevalue", Converter::Missing),
    ("ifcderivedprofiledef", Converter::Class(Class::IfcDerivedProfileDef)),
    ("ifcderivedunit", Converter::NotImplemented),
    ("ifcderivedunitelement", Converter::NotImplemented),
    ("ifcderivedunitenum", Converter::Missing),
    ("ifcdescriptivemeasure", Converter::Missing),
    ("ifcdiameterdimension", Converter::Class(Class::IfcDiameterDimension)),
    ("ifcdimensionalexponents", Converter::NotImplemented),
    ("ifcdimensioncalloutrelationship", Converter::NotImplemented),
    ("ifcdimensioncount", Converter::Missing),
    ("ifcdimensioncurve", Converter::Class(Class::IfcDimensionCurve)),
    ("ifcdimensioncurvedirectedcallout", Converter::Class(Class::IfcDimensionCurveDirectedCallout)),
    ("ifcdimensioncurveterminator", Converter::Class(Class::IfcDimensionCurveTerminator)),
    ("ifcdimensionextentusage", Converter::Missing),
    ("ifcdimensionpair", Converter::NotImplemented),
    ("ifcdirection", Converter::Class(Class::IfcDirection)),
    ("ifcdirectionsenseenum", Converter::Missing),
    ("ifcdiscreteaccessory", Converter::Class(Class::IfcDiscreteAccessory)),
    ("ifcdiscreteaccessorytype", Converter::Class(Class::IfcDiscreteAccessoryType)),
    ("ifcdistributionchamberelement", Converter::Class(Class::IfcDistributionChamberElement)),
    ("ifcdistributionchamberelementtype", Converter::Class(Class::IfcDistributionChamberElementType)),
    ("ifcdistributionchamberelementtypeenum", Converter::Missing),
    ("ifcdistributioncontrolelement", Converter::Class(Class::IfcDistributionControlElement)),
    ("ifcdistributioncontrolelementtype", Converter::Class(Class::IfcDistributionControlElementType)),
    ("ifcdistributionelement", Converter::Class(Class::IfcDistributionElement)),
    ("ifcdistributionelementtype", Converter::Class(Class::IfcDistributionElementType)),
    ("ifcdistributionflowelement", Converter::Class(Class::IfcDistributionFlowElement)),
    ("ifcdistributionflowelementtype", Converter::Class(Class::IfcDistributionFlowElementType)),
    ("ifcdistributionport", Converter::Class(Class::IfcDistributionPort)),
    ("ifcdocumentconfidentialityenum", Converter::Missing),
    ("ifcdocumentelectronicformat", Converter::NotImplemented),
    ("ifcdocumentinformation", Converter::NotImplemented),
    ("ifcdocumentinformationrelationship", Converter::NotImplemented),
    ("ifcdocumentreference", Converter::NotImplemented),
    ("ifcdocumentselect", Converter::Missing),
    ("ifcdocumentstatusenum", Converter::Missing),
    ("ifcdoor", Converter::Class(Class::IfcDoor)),
    ("ifcdoorliningproperties", Converter::NotImplemented),
    ("ifcdoorpaneloperationenum", Converter::Missing),
    ("ifcdoorpanelpositionenum", Converter::Missing),
    ("ifcdoorpanelproperties", Converter::NotImplemented),
    ("ifcdoorstyle", Converter::Class(Class::IfcDoorStyle)),
    ("ifcdoorstyleconstructionenum", Converter::Missing),
    ("ifcdoorstyleoperationenum", Converter::Missing),
    ("ifcdoseequivalentmeasure", Converter::Missing),
    ("ifcdraughtingcallout", Converter::Class(Class::IfcDraughtingCallout)),
    ("ifcdraughtingcalloutelement", Converter::Missing),
    ("ifcdraughtingcalloutrelationship", Converter::NotImplemented),
    ("ifcdraughtingpredefinedcolour", Converter::NotImplemented),
    ("ifcdraughtingpredefinedcurvefont", Converter::NotImplemented),
    ("ifcdraughtingpredefinedtextfont", Converter::NotImplemented),
    ("ifcductfittingtype", Converter::Class(Class::IfcDuctFittingType)),
    ("ifcductfittingtypeenum", Converter::Missing),
    ("ifcductsegmenttype", Converter::Class(Class::IfcDuctSegmentType)),
    ("ifcductsegmenttypeenum", Converter::Missing),
    ("ifcductsilencertype", Converter::Class(Class::IfcDuctSilencerType)),
    ("ifcductsilencertypeenum", Converter::Missing),
    ("ifcdynamicviscositymeasure", Converter::Missing),
    ("ifcedge", Converter::Class(Class::IfcEdge)),
    ("ifcedgecurve", Converter::Class(Class::IfcEdgeCurve)),
    ("ifcedgefeature", Converter::Class(Class::IfcEdgeFeature)),
    ("ifcedgeloop", Converter::Class(Class::IfcEdgeLoop)),
    ("ifcelectricalbaseproperties", Converter::NotImplemented),
    ("ifcelectricalcircuit", Converter::Class(Class::IfcElectricalCircuit)),
    ("ifcelectricalelement", Converter::Class(Class::IfcElectricalElement)),
    ("ifcelectricappliancetype", Converter::Class(Class::IfcElectricApplianceType)),
    ("ifcelectricappliancetypeenum", Converter::Missing),
    ("ifcelectriccapacitancemeasure", Converter::Missing),
    ("ifcelectricchargemeasure", Converter::Missing),
    ("ifcelectricconductancemeasure", Converter::Missing),
    ("ifcelectriccurrentenum", Converter::Missing),
    ("ifcelectriccurrentmeasure", Converter::Missing),
    ("ifcelectricdistributionpoint", Converter::Class(Class::IfcElectricDistributionPoint)),
    ("ifcelectricdistributionpointfunctionenum", Converter::Missing),
    ("ifcelectricflowstoragedevicetype", Converter::Class(Class::IfcElectricFlowStorageDeviceType)),
    ("ifcelectricflowstoragedevicetypeenum", Converter::Missing),
    ("ifcelectricgeneratortype", Converter::Class(Class::IfcElectricGeneratorType)),
    ("ifcelectricgeneratortypeenum", Converter::Missing),
    ("ifcelectricheatertype", Converter::Class(Class::IfcElectricHeaterType)),
    ("ifcelectricheatertypeenum", Converter::Missing),
    ("ifcelectricmotortype", Converter::Class(Class::IfcElectricMotorType)),
    ("ifcelectricmotortypeenum", Converter::Missing),
    ("ifcelectricresistancemeasure", Converter::Missing),
    ("ifcelectrictimecontroltype", Converter::Class(Class::IfcElectricTimeControlType)),
    ("ifcelectrictimecontroltypeenum", Converter::Missing),
    ("ifcelectricvoltagemeasure", Converter::Missing),
    ("ifcelement", Converter::Class(Class::IfcElement)),
    ("ifcelementarysurface", Converter::Class(Class::IfcElementarySurface)),
    ("ifcelementassembly", Converter::Class(Class::IfcElementAssembly)),
    ("ifcelementassemblytypeenum", Converter::Missing),
    ("ifcelementcomponent", Converter::Class(Class::IfcElementComponent)),
    ("ifcelementcomponenttype", Converter::Class(Class::IfcElementComponentType)),
    ("ifcelementcompositionenum", Converter::Missing),
    ("ifcelementquantity", Converter::Class(Class::IfcElementQuantity)),
    ("ifcelementtype", Converter::Class(Class::IfcElementType)),
    ("ifcellipse", Converter::Class(Class::IfcEllipse)),
    ("ifcellipseprofiledef", Converter::Class(Class::IfcEllipseProfileDef)),
    ("ifcenergyconversiondevice", Converter::Class(Class::IfcEnergyConversionDevice)),
    ("ifcenergyconversiondevicetype", Converter::Class(Class::IfcEnergyConversionDeviceType)),
    ("ifcenergymeasure", Converter::Missing),
    ("ifcenergyproperties", Converter::NotImplemented),
    ("ifcenergysequenceenum", Converter::Missing),
    ("ifcenvironmentalimpactcategoryenum", Converter::Missing),
    ("ifcenvironmentalimpactvalue", Converter::NotImplemented),
    ("ifcequipmentelement", Converter::Class(Class::IfcEquipmentElement)),
    ("ifcequipmentstandard", Converter::Class(Class::IfcEquipmentStandard)),
    ("ifcevaporativecoolertype", Converter::Class(Class::IfcEvaporativeCoolerType)),
    ("ifcevaporativecoolertypeenum", Converter::Missing),
    ("ifcevaporatortype", Converter::Class(Class::IfcEvaporatorType)),
    ("ifcevaporatortypeenum", Converter::Missing),
    ("ifcextendedmaterialproperties", Converter::NotImplemented),
    ("ifcexternallydefinedhatchstyle", Converter::NotImplemented),
    ("ifcexternallydefinedsurfacestyle", Converter::NotImplemented),
    ("ifcexternallydefinedsymbol", Converter::NotImplemented),
    ("ifcexternallydefinedtextfont", Converter::NotImplemented),
    ("ifcexternalreference", Converter::NotImplemented),
    ("ifcextrudedareasolid", Converter::Class(Class::IfcExtrudedAreaSolid)),
    ("ifcface", Converter::Class(Class::IfcFace)),
    ("ifcfacebasedsurfacemodel", Converter::Class(Class::IfcFaceBasedSurfaceModel)),
    ("ifcfacebound", Converter::Class(Class::IfcFaceBound)),
    ("ifcfaceouterbound", Converter::Class(Class::IfcFaceOuterBound)),
    ("ifcfacesurface", Converter::Class(Class::IfcFaceSurface)),
    ("ifcfacetedbrep", Converter::Class(Class::IfcFacetedBrep)),
    ("ifcfacetedbrepwithvoids", Converter::Class(Class::IfcFacetedBrepWithVoids)),
    ("ifcfailureconnectioncondition", Converter::NotImplemented),
    ("ifcfantype", Converter::Class(Class::IfcFanType)),
    ("ifcfantypeenum", Converter::Missing),
    ("ifcfastener", Converter::Class(Class::IfcFastener)),
    ("ifcfastenertype", Converter::Class(Class::IfcFastenerType)),
    ("ifcfeatureelement", Converter::Class(Class::IfcFeatureElement)),
    ("ifcfeatureelementaddition", Converter::Class(Class::IfcFeatureElementAddition)),
    ("ifcfeatureelementsubtraction", Converter::Class(Class::IfcFeatureElementSubtraction)),
    ("ifcfillareastyle", Converter::NotImplemented),
    ("ifcfillareastylehatching", Converter::Class(Class::IfcFillAreaStyleHatching)),
    ("ifcfillareastyletiles", Converter::Class(Class::IfcFillAreaStyleTiles)),
    ("ifcfillareastyletileshapeselect", Converter::Missing),
    ("ifcfillareastyletilesymbolwithstyle", Converter::Class(Class::IfcFillAreaStyleTileSymbolWithStyle)),
    ("ifcfillstyleselect", Converter::Missing),
    ("ifcfiltertype", Converter::Class(Class::IfcFilterType)),
    ("ifcfiltertypeenum", Converter::Missing),
    ("ifcfiresuppressionterminaltype", Converter::Class(Class::IfcFireSuppressionTerminalType)),
    ("ifcfiresuppressionterminaltypeenum", Converter::Missing),
    ("ifcflowcontroller", Converter::Class(Class::IfcFlowController)),
    ("ifcflowcontrollertype", Converter::Class(Class::IfcFlowControllerType)),
    ("ifcflowdirectionenum", Converter::Missing),
    ("ifcflowfitting", Converter::Class(Class::IfcFlowFitting)),
    ("ifcflowfittingtype", Converter::Class(Class::IfcFlowFittingType)),
    ("ifcflowinstrumenttype", Converter::Class(Class::IfcFlowInstrumentType)),
    ("ifcflowinstrumenttypeenum", Converter::Missing),
    ("ifcflowmetertype", Converter::Class(Class::IfcFlowMeterType)),
    ("ifcflowmetertypeenum", Converter::Missing),
    ("ifcflowmovingdevice", Converter::Class(Class::IfcFlowMovingDevice)),
    ("ifcflowmovingdevicetype", Converter::Class(Class::IfcFlowMovingDeviceType)),
    ("ifcflowsegment", Converter::Class(Class::IfcFlowSegment)),
    ("ifcflowsegmenttype", Converter::Class(Class::IfcFlowSegmentType)),
    ("ifcflowstoragedevice", Converter::Class(Class::IfcFlowStorageDevice)),
    ("ifcflowstoragedevicetype", Converter::Class(Class::IfcFlowStorageDeviceType)),
    ("ifcflowterminal", Converter::Class(Class::IfcFlowTerminal)),
    ("ifcflowterminaltype", Converter::Class(Class::IfcFlowTerminalType)),
    ("ifcflowtreatmentdevice", Converter::Class(Class::IfcFlowTreatmentDevice)),
    ("ifcflowtreatmentdevicetype", Converter::Class(Class::IfcFlowTreatmentDeviceType)),
    ("ifcfluidflowproperties", Converter::NotImplemented),
    ("ifcfontstyle", Converter::Missing),
    ("ifcfontvariant", Converter::Missing),
    ("ifcfontweight", Converter::Missing),
    ("ifcfooting", Converter::Class(Class::IfcFooting)),
    ("ifcfootingtypeenum", Converter::Missing),
    ("ifcforcemeasure", Converter::Missing),
    ("ifcfrequencymeasure", Converter::Missing),
    ("ifcfuelproperties", Converter::NotImplemented),
    ("ifcfurnishingelement", Converter::Class(Class::IfcFurnishingElement)),
    ("ifcfurnishingelementtype", Converter::Class(Class::IfcFurnishingElementType)),
    ("ifcfurniturestandard", Converter::Class(Class::IfcFurnitureStandard)),
    ("ifcfurnituretype", Converter::Class(Class::IfcFurnitureType)),
    ("ifcgasterminaltype", Converter::Class(Class::IfcGasTerminalType)),
    ("ifcgasterminaltypeenum", Converter::Missing),
    ("ifcgeneralmaterialproperties", Converter::NotImplemented),
    ("ifcgeneralprofileproperties", Converter::NotImplemented),
    ("ifcgeometriccurveset", Converter::Class(Class::IfcGeometricCurveSet)),
    ("ifcgeometricprojectionenum", Converter::Missing),
    ("ifcgeometricrepresentationcontext", Converter::Class(Class::IfcGeometricRepresentationContext)),
    ("ifcgeometricrepresentationitem", Converter::Class(Class::IfcGeometricRepresentationItem)),
    ("ifcgeometricrepresentationsubcontext", Converter::Class(Class::IfcGeometricRepresentationSubContext)),
    ("ifcgeometricset", Converter::Class(Class::IfcGeometricSet)),
    ("ifcgeometricsetselect", Converter::Missing),
    ("ifcgloballyuniqueid", Converter::Missing),
    ("ifcglobalorlocalenum", Converter::Missing),
    ("ifcgrid", Converter::Class(Class::IfcGrid)),
    ("ifcgridaxis", Converter::NotImplemented),
    ("ifcgridplacement", Converter::Class(Class::IfcGridPlacement)),
    ("ifcgroup", Converter::Class(Class::IfcGroup)),
    ("ifchalfspacesolid", Converter::Class(Class::IfcHalfSpaceSolid)),
    ("ifchatchlinedistanceselect", Converter::Missing),
    ("ifcheatexchangertype", Converter::Class(Class::IfcHeatExchangerType)),
    ("ifcheatexchangertypeenum", Converter::Missing),
    ("ifcheatfluxdensitymeasure", Converter::Missing),
    ("ifcheatingvaluemeasure", Converter::Missing),
    ("ifchourinday", Converter::Missing),
    ("ifchumidifiertype", Converter::Class(Class::IfcHumidifierType)),
    ("ifchumidifiertypeenum", Converter::Missing),
    ("ifchygroscopicmaterialproperties", Converter::NotImplemented),
    ("ifcidentifier", Converter::Missing),
    ("ifcilluminancemeasure", Converter::Missing),
    ("ifcimagetexture", Converter::NotImplemented),
    ("ifcinductancemeasure", Converter::Missing),
    ("ifcinteger", Converter::Missing),
    ("ifcintegercountratemeasure", Converter::Missing),
    ("ifcinternalorexternalenum", Converter::Missing),
    ("ifcinventory", Converter::Class(Class::IfcInventory)),
    ("ifcinventorytypeenum", Converter::Missing),
    ("ifcionconcentrationmeasure", Converter::Missing),
    ("ifcirregulartimeseries", Converter::NotImplemented),
    ("ifcirregulartimeseriesvalue", Converter::NotImplemented),
    ("ifcishapeprofiledef", Converter::Class(Class::IfcIShapeProfileDef)),
    ("ifcisothermalmoisturecapacitymeasure", Converter::Missing),
    ("ifcjunctionboxtype", Converter::Class(Class::IfcJunctionBoxType)),
    ("ifcjunctionboxtypeenum", Converter::Missing),
    ("ifckinematicviscositymeasure", Converter::Missing),
    ("ifclabel", Converter::Missing),
    ("ifclaborresource", Converter::Class(Class::IfcLaborResource)),
    ("ifclamptype", Converter::Class(Class::IfcLampType)),
    ("ifclamptypeenum", Converter::Missing),
    ("ifclayereditem", Converter::Missing),
    ("ifclayersetdirectionenum", Converter::Missing),
    ("ifclengthmeasure", Converter::Missing),
    ("ifclibraryinformation", Converter::NotImplemented),
    ("ifclibraryreference", Converter::NotImplemented),
    ("ifclibraryselect", Converter::Missing),
    ("ifclightdistributioncurveenum", Converter::Missing),
    ("ifclightdistributiondata", Converter::NotImplemented),
    ("ifclightdistributiondatasourceselect", Converter::Missing),
    ("ifclightemissionsourceenum", Converter::Missing),
    ("ifclightfixturetype", Converter::Class(Class::IfcLightFixtureType)),
    ("ifclightfixturetypeenum", Converter::Missing),
    ("ifclightintensitydistribution", Converter::NotImplemented),
    ("ifclightsource", Converter::Class(Class::IfcLightSource)),
    ("ifclightsourceambient", Converter::Class(Class::IfcLightSourceAmbient)),
    ("ifclightsourcedirectional", Converter::Class(Class::IfcLightSourceDirectional)),
    ("ifclightsourcegoniometric", Converter::Class(Class::IfcLightSourceGoniometric)),
    ("ifclightsourcepositional", Converter::Class(Class::IfcLightSourcePositional)),
    ("ifclightsourcespot", Converter::Class(Class::IfcLightSourceSpot)),
    ("ifcline", Converter::Class(Class::IfcLine)),
    ("ifclineardimension", Converter::Class(Class::IfcLinearDimension)),
    ("ifclinearforcemeasure", Converter::Missing),
    ("ifclinearmomentmeasure", Converter::Missing),
    ("ifclinearstiffnessmeasure", Converter::Missing),
    ("ifclinearvelocitymeasure", Converter::Missing),
    ("ifcloadgrouptypeenum", Converter::Missing),
    ("ifclocalplacement", Converter::Class(Class::IfcLocalPlacement)),
    ("ifclocaltime", Converter::NotImplemented),
    ("ifclogical", Converter::Missing),
    ("ifclogicaloperatorenum", Converter::Missing),
    ("ifcloop", Converter::Class(Class::IfcLoop)),
    ("ifclshapeprofiledef", Converter::Class(Class::IfcLShapeProfileDef)),
    ("ifcluminousfluxmeasure", Converter::Missing),
    ("ifcluminousintensitydistributionmeasure", Converter::Missing),
    ("ifcluminousintensitymeasure", Converter::Missing),
    ("ifcmagneticfluxdensitymeasure", Converter::Missing),
    ("ifcmagneticfluxmeasure", Converter::Missing),
    ("ifcmanifoldsolidbrep", Converter::Class(Class::IfcManifoldSolidBrep)),
    ("ifcmappeditem", Converter::Class(Class::IfcMappedItem)),
    ("ifcmassdensitymeasure", Converter::Missing),
    ("ifcmassflowratemeasure", Converter::Missing),
    ("ifcmassmeasure", Converter::Missing),
    ("ifcmassperlengthmeasure", Converter::Missing),
    ("ifcmaterial", Converter::NotImplemented),
    ("ifcmaterialclassificationrelationship", Converter::NotImplemented),
    ("ifcmaterialdefinitionrepresentation", Converter::Class(Class::IfcMaterialDefinitionRepresentation)),
    ("ifcmateriallayer", Converter::NotImplemented),
    ("ifcmateriallayerset", Converter::NotImplemented),
    ("ifcmateriallayersetusage", Converter::NotImplemented),
    ("ifcmateriallist", Converter::NotImplemented),
    ("ifcmaterialproperties", Converter::NotImplemented),
    ("ifcmaterialselect", Converter::Missing),
    ("ifcmeasurevalue", Converter::Missing),
    ("ifcmeasurewithunit", Converter::Class(Class::IfcMeasureWithUnit)),
    ("ifcmechanicalconcretematerialproperties", Converter::NotImplemented),
    ("ifcmechanicalfastener", Converter::Class(Class::IfcMechanicalFastener)),
    ("ifcmechanicalfastenertype", Converter::Class(Class::IfcMechanicalFastenerType)),
    ("ifcmechanicalmaterialproperties", Converter::NotImplemented),
    ("ifcmechanicalsteelmaterialproperties", Converter::NotImplemented),
    ("ifcmember", Converter::Class(Class::IfcMember)),
    ("ifcmembertype", Converter::Class(Class::IfcMemberType)),
    ("ifcmembertypeenum", Converter::Missing),
    ("ifcmetric", Converter::NotImplemented),
    ("ifcmetricvalueselect", Converter::Missing),
    ("ifcminuteinhour", Converter::Missing),
    ("ifcmodulusofelasticitymeasure", Converter::Missing),
    ("ifcmodulusoflinearsubgradereactionmeasure", Converter::Missing),
    ("ifcmodulusofrotationalsubgradereactionmeasure", Converter::Missing),
    ("ifcmodulusofsubgradereactionmeasure", Converter::Missing),
    ("ifcmoisturediffusivitymeasure", Converter::Missing),
    ("ifcmolecularweightmeasure", Converter::Missing),
    ("ifcmomentofinertiameasure", Converter::Missing),
    ("ifcmonetarymeasure", Converter::Missing),
    ("ifcmonetaryunit", Converter::NotImplemented),
    ("ifcmonthinyearnumber", Converter::Missing),
    ("ifcmotorconnectiontype", Converter::Class(Class::IfcMotorConnectionType)),
    ("ifcmotorconnectiontypeenum", Converter::Missing),
    ("ifcmove", Converter::Class(Class::IfcMove)),
    ("ifcnamedunit", Converter::Class(Class::IfcNamedUnit)),
    ("ifcnormalisedratiomeasure", Converter::Missing),
    ("ifcnullstyle", Converter::Missing),
    ("ifcnumericmeasure", Converter::Missing),
    ("ifcobject", Converter::Class(Class::IfcObject)),
    ("ifcobjectdefinition", Converter::Class(Class::IfcObjectDefinition)),
    ("ifcobjective", Converter::NotImplemented),
    ("ifcobjectiveenum", Converter::Missing),
    ("ifcobjectplacement", Converter::Class(Class::IfcObjectPlacement)),
    ("ifcobjectreferenceselect", Converter::Missing),
    ("ifcobjecttypeenum", Converter::Missing),
    ("ifcoccupant", Converter::Class(Class::IfcOccupant)),
    ("ifcoccupanttypeenum", Converter::Missing),
    ("ifcoffsetcurve2d", Converter::Class(Class::IfcOffsetCurve2D)),
    ("ifcoffsetcurve3d", Converter::Class(Class::IfcOffsetCurve3D)),
    ("ifconedirectionrepeatfactor", Converter::Class(Class::IfcOneDirectionRepeatFactor)),
    ("ifcopeningelement", Converter::Class(Class::IfcOpeningElement)),
    ("ifcopenshell", Converter::Class(Class::IfcOpenShell)),
    ("ifcopticalmaterialproperties", Converter::NotImplemented),
    ("ifcorderaction", Converter::Class(Class::IfcOrderAction)),
    ("ifcorganization", Converter::NotImplemented),
    ("ifcorganizationrelationship", Converter::NotImplemented),
    ("ifcorientationselect", Converter::Missing),
    ("ifcorientededge", Converter::Class(Class::IfcOrientedEdge)),
    ("ifcoutlettype", Converter::Class(Class::IfcOutletType)),
    ("ifcoutlettypeenum", Converter::Missing),
    ("ifcownerhistory", Converter::NotImplemented),
    ("ifcparameterizedprofiledef", Converter::Class(Class::IfcParameterizedProfileDef)),
    ("ifcparametervalue", Converter::Missing),
    ("ifcpath", Converter::Class(Class::IfcPath)),
    ("ifcperformancehistory", Converter::Class(Class::IfcPerformanceHistory)),
    ("ifcpermeablecoveringoperationenum", Converter::Missing),
    ("ifcpermeablecoveringproperties", Converter::NotImplemented),
    ("ifcpermit", Converter::Class(Class::IfcPermit)),
    ("ifcperson", Converter::NotImplemented),
    ("ifcpersonandorganization", Converter::NotImplemented),
    ("ifcphmeasure", Converter::Missing),
    ("ifcphysicalcomplexquantity", Converter::NotImplemented),
    ("ifcphysicalorvirtualenum", Converter::Missing),
    ("ifcphysicalquantity", Converter::NotImplemented),
    ("ifcphysicalsimplequantity", Converter::NotImplemented),
    ("ifcpile", Converter::Class(Class::IfcPile)),
    ("ifcpileconstructionenum", Converter::Missing),
    ("ifcpiletypeenum", Converter::Missing),
    ("ifcpipefittingtype", Converter::Class(Class::IfcPipeFittingType)),
    ("ifcpipefittingtypeenum", Converter::Missing),
    ("ifcpipesegmenttype", Converter::Class(Class::IfcPipeSegmentType)),
    ("ifcpipesegmenttypeenum", Converter::Missing),
    ("ifcpixeltexture", Converter::NotImplemented),
    ("ifcplacement", Converter::Class(Class::IfcPlacement)),
    ("ifcplanarbox", Converter::Class(Class::IfcPlanarBox)),
    ("ifcplanarextent", Converter::Class(Class::IfcPlanarExtent)),
    ("ifcplanarforcemeasure", Converter::Missing),
    ("ifcplane", Converter::Class(Class::IfcPlane)),
    ("ifcplaneanglemeasure", Converter::Missing),
    ("ifcplate", Converter::Class(Class::IfcPlate)),
    ("ifcplatetype", Converter::Class(Class::IfcPlateType)),
    ("ifcplatetypeenum", Converter::Missing),
    ("ifcpoint", Converter::Class(Class::IfcPoint)),
    ("ifcpointoncurve", Converter::Class(Class::IfcPointOnCurve)),
    ("ifcpointonsurface", Converter::Class(Class::IfcPointOnSurface)),
    ("ifcpointorvertexpoint", Converter::Missing),
    ("ifcpolygonalboundedhalfspace", Converter::Class(Class::IfcPolygonalBoundedHalfSpace)),
    ("ifcpolyline", Converter::Class(Class::IfcPolyline)),
    ("ifcpolyloop", Converter::Class(Class::IfcPolyLoop)),
    ("ifcport", Converter::Class(Class::IfcPort)),
    ("ifcpositivelengthmeasure", Converter::Missing),
    ("ifcpositiveplaneanglemeasure", Converter::Missing),
    ("ifcpositiveratiomeasure", Converter::Missing),
    ("ifcpostaladdress", Converter::NotImplemented),
    ("ifcpowermeasure", Converter::Missing),
    ("ifcpredefinedcolour", Converter::NotImplemented),
    ("ifcpredefinedcurvefont", Converter::NotImplemented),
    ("ifcpredefineddimensionsymbol", Converter::NotImplemented),
    ("ifcpredefineditem", Converter::NotImplemented),
    ("ifcpredefinedpointmarkersymbol", Converter::NotImplemented),
    ("ifcpredefinedsymbol", Converter::NotImplemented),
    ("ifcpredefinedterminatorsymbol", Converter::NotImplemented),
    ("ifcpredefinedtextfont", Converter::NotImplemented),
    ("ifcpresentabletext", Converter::Missing),
    ("ifcpresentationlayerassignment", Converter::NotImplemented),
    ("ifcpresentationlayerwithstyle", Converter::NotImplemented),
    ("ifcpresentationstyle", Converter::Class(Class::IfcPresentationStyle)),
    ("ifcpresentationstyleassignment", Converter::Class(Class::IfcPresentationStyleAssignment)),
    ("ifcpresentationstyleselect", Converter::Missing),
    ("ifcpressuremeasure", Converter::Missing),
    ("ifcprocedure", Converter::Class(Class::IfcProcedure)),
    ("ifcproceduretypeenum", Converter::Missing),
    ("ifcprocess", Converter::Class(Class::IfcProcess)),
    ("ifcproduct", Converter::Class(Class::IfcProduct)),
    ("ifcproductdefinitionshape", Converter::Class(Class::IfcProductDefinitionShape)),
    ("ifcproductrepresentation", Converter::Class(Class::IfcProductRepresentation)),
    ("ifcproductsofcombustionproperties", Converter::NotImplemented),
    ("ifcprofiledef", Converter::Class(Class::IfcProfileDef)),
    ("ifcprofileproperties", Converter::NotImplemented),
    ("ifcprofiletypeenum", Converter::Missing),
    ("ifcproject", Converter::Class(Class::IfcProject)),
    ("ifcprojectedortruelengthenum", Converter::Missing),
    ("ifcprojectioncurve", Converter::Class(Class::IfcProjectionCurve)),
    ("ifcprojectionelement", Converter::Class(Class::IfcProjectionElement)),
    ("ifcprojectorder", Converter::Class(Class::IfcProjectOrder)),
    ("ifcprojectorderrecord", Converter::Class(Class::IfcProjectOrderRecord)),
    ("ifcprojectorderrecordtypeenum", Converter::Missing),
    ("ifcprojectordertypeenum", Converter::Missing),
    ("ifcproperty", Converter::Class(Class::IfcProperty)),
    ("ifcpropertyboundedvalue", Converter::Class(Class::IfcPropertyBoundedValue)),
    ("ifcpropertyconstraintrelationship", Converter::NotImplemented),
    ("ifcpropertydefinition", Converter::Class(Class::IfcPropertyDefinition)),
    ("ifcpropertydependencyrelationship", Converter::NotImplemented),
    ("ifcpropertyenumeratedvalue", Converter::Class(Class::IfcPropertyEnumeratedValue)),
    ("ifcpropertyenumeration", Converter::NotImplemented),
    ("ifcpropertylistvalue", Converter::Class(Class::IfcPropertyListValue)),
    ("ifcpropertyreferencevalue", Converter::Class(Class::IfcPropertyReferenceValue)),
    ("ifcpropertyset", Converter::Class(Class::IfcPropertySet)),
    ("ifcpropertysetdefinition", Converter::Class(Class::IfcPropertySetDefinition)),
    ("ifcpropertysinglevalue", Converter::Class(Class::IfcPropertySingleValue)),
    ("ifcpropertysourceenum", Converter::Missing),
    ("ifcpropertytablevalue", Converter::Class(Class::IfcPropertyTableValue)),
    ("ifcprotectivedevicetype", Converter::Class(Class::IfcProtectiveDeviceType)),
    ("ifcprotectivedevicetypeenum", Converter::Missing),
    ("ifcproxy", Converter::Class(Class::IfcProxy)),
    ("ifcpumptype", Converter::Class(Class::IfcPumpType)),
    ("ifcpumptypeenum", Converter::Missing),
    ("ifcquantityarea", Converter::NotImplemented),
    ("ifcquantitycount", Converter::NotImplemented),
    ("ifcquantitylength", Converter::NotImplemented),
    ("ifcquantitytime", Converter::NotImplemented),
    ("ifcquantityvolume", Converter::NotImplemented),
    ("ifcquantityweight", Converter::NotImplemented),
    ("ifcradioactivitymeasure", Converter::Missing),
    ("ifcradiusdimension", Converter::Class(Class::IfcRadiusDimension)),
    ("ifcrailing", Converter::Class(Class::IfcRailing)),
    ("ifcrailingtype", Converter::Class(Class::IfcRailingType)),
    ("ifcrailingtypeenum", Converter::Missing),
    ("ifcramp", Converter::Class(Class::IfcRamp)),
    ("ifcrampflight", Converter::Class(Class::IfcRampFlight)),
    ("ifcrampflighttype", Converter::Class(Class::IfcRampFlightType)),
    ("ifcrampflighttypeenum", Converter::Missing),
    ("ifcramptypeenum", Converter::Missing),
    ("ifcratiomeasure", Converter::Missing),
    ("ifcrationalbeziercurve", Converter::Class(Class::IfcRationalBezierCurve)),
    ("ifcreal", Converter::Missing),
    ("ifcrectanglehollowprofiledef", Converter::Class(Class::IfcRectangleHollowProfileDef)),
    ("ifcrectangleprofiledef", Converter::Class(Class::IfcRectangleProfileDef)),
    ("ifcrectangularpyramid", Converter::Class(Class::IfcRectangularPyramid)),
    ("ifcrectangulartrimmedsurface", Converter::Class(Class::IfcRectangularTrimmedSurface)),
    ("ifcreferencesvaluedocument", Converter::NotImplemented),
    ("ifcreflectancemethodenum", Converter::Missing),
    ("ifcregulartimeseries", Converter::NotImplemented),
    ("ifcreinforcementbarproperties", Converter::NotImplemented),
    ("ifcreinforcementdefinitionproperties", Converter::NotImplemented),
    ("ifcreinforcingbar", Converter::Class(Class::IfcReinforcingBar)),
    ("ifcreinforcingbarroleenum", Converter::Missing),
    ("ifcreinforcingbarsurfaceenum", Converter::Missing),
    ("ifcreinforcingelement", Converter::Class(Class::IfcReinforcingElement)),
    ("ifcreinforcingmesh", Converter::Class(Class::IfcReinforcingMesh)),
    ("ifcrelaggregates", Converter::Class(Class::IfcRelAggregates)),
    ("ifcrelassigns", Converter::NotImplemented),
    ("ifcrelassignstasks", Converter::NotImplemented),
    ("ifcrelassignstoactor", Converter::NotImplemented),
    ("ifcrelassignstocontrol", Converter::NotImplemented),
    ("ifcrelassignstogroup", Converter::NotImplemented),
    ("ifcrelassignstoprocess", Converter::NotImplemented),
    ("ifcrelassignstoproduct", Converter::NotImplemented),
    ("ifcrelassignstoprojectorder", Converter::NotImplemented),
    ("ifcrelassignstoresource", Converter::NotImplemented),
    ("ifcrelassociates", Converter::NotImplemented),
    ("ifcrelassociatesappliedvalue", Converter::NotImplemented),
    ("ifcrelassociatesapproval", Converter::NotImplemented),
    ("ifcrelassociatesclassification", Converter::NotImplemented),
    ("ifcrelassociatesconstraint", Converter::NotImplemented),
    ("ifcrelassociatesdocument", Converter::NotImplemented),
    ("ifcrelassociateslibrary", Converter::NotImplemented),
    ("ifcrelassociatesmaterial", Converter::NotImplemented),
    ("ifcrelassociatesprofileproperties", Converter::NotImplemented),
    ("ifcrelationship", Converter::Class(Class::IfcRelationship)),
    ("ifcrelaxation", Converter::NotImplemented),
    ("ifcrelconnects", Converter::Class(Class::IfcRelConnects)),
    ("ifcrelconnectselements", Converter::NotImplemented),
    ("ifcrelconnectspathelements", Converter::NotImplemented),
    ("ifcrelconnectsports", Converter::NotImplemented),
    ("ifcrelconnectsporttoelement", Converter::NotImplemented),
    ("ifcrelconnectsstructuralactivity", Converter::NotImplemented),
    ("ifcrelconnectsstructuralelement", Converter::NotImplemented),
    ("ifcrelconnectsstructuralmember", Converter::NotImplemented),
    ("ifcrelconnectswitheccentricity", Converter::NotImplemented),
    ("ifcrelconnectswithrealizingelements", Converter::NotImplemented),
    ("ifcrelcontainedinspatialstructure", Converter::Class(Class::IfcRelContainedInSpatialStructure)),
    ("ifcrelcoversbldgelements", Converter::NotImplemented),
    ("ifcrelcoversspaces", Converter::NotImplemented),
    ("ifcreldecomposes", Converter::Class(Class::IfcRelDecomposes)),
    ("ifcreldefines", Converter::Class(Class::IfcRelDefines)),
    ("ifcreldefinesbyproperties", Converter::Class(Class::IfcRelDefinesByProperties)),
    ("ifcreldefinesbytype", Converter::NotImplemented),
    ("ifcrelfillselement", Converter::Class(Class::IfcRelFillsElement)),
    ("ifcrelflowcontrolelements", Converter::NotImplemented),
    ("ifcrelinteractionrequirements", Converter::NotImplemented),
    ("ifcrelnests", Converter::NotImplemented),
    ("ifcreloccupiesspaces", Converter::NotImplemented),
    ("ifcreloverridesproperties", Converter::Class(Class::IfcRelOverridesProperties)),
    ("ifcrelprojectselement", Converter::NotImplemented),
    ("ifcrelreferencedinspatialstructure", Converter::NotImplemented),
    ("ifcrelschedulescostitems", Converter::NotImplemented),
    ("ifcrelsequence", Converter::NotImplemented),
    ("ifcrelservicesbuildings", Converter::NotImplemented),
    ("ifcrelspaceboundary", Converter::NotImplemented),
    ("ifcrelvoidselement", Converter::Class(Class::IfcRelVoidsElement)),
    ("ifcrepresentation", Converter::Class(Class::IfcRepresentation)),
    ("ifcrepresentationcontext", Converter::Class(Class::IfcRepresentationContext)),
    ("ifcrepresentationitem", Converter::Class(Class::IfcRepresentationItem)),
    ("ifcrepresentationmap", Converter::Class(Class::IfcRepresentationMap)),
    ("ifcresource", Converter::Class(Class::IfcResource)),
    ("ifcresourceconsumptionenum", Converter::Missing),
    ("ifcrevolvedareasolid", Converter::Class(Class::IfcRevolvedAreaSolid)),
    ("ifcribplatedirectionenum", Converter::Missing),
    ("ifcribplateprofileproperties", Converter::NotImplemented),
    ("ifcrightcircularcone", Converter::Class(Class::IfcRightCircularCone)),
    ("ifcrightcircularcylinder", Converter::Class(Class::IfcRightCircularCylinder)),
    ("ifcroleenum", Converter::Missing),
    ("ifcroof", Converter::Class(Class::IfcRoof)),
    ("ifcrooftypeenum", Converter::Missing),
    ("ifcroot", Converter::Class(Class::IfcRoot)),
    ("ifcrotationalfrequencymeasure", Converter::Missing),
    ("ifcrotationalmassmeasure", Converter::Missing),
    ("ifcrotationalstiffnessmeasure", Converter::Missing),
    ("ifcroundededgefeature", Converter::Class(Class::IfcRoundedEdgeFeature)),
    ("ifcroundedrectangleprofiledef", Converter::Class(Class::IfcRoundedRectangleProfileDef)),
    ("ifcsanitaryterminaltype", Converter::Class(Class::IfcSanitaryTerminalType)),
    ("ifcsanitaryterminaltypeenum", Converter::Missing),
    ("ifcscheduletimecontrol", Converter::Class(Class::IfcScheduleTimeControl)),
    ("ifcsecondinminute", Converter::Missing),
    ("ifcsectionalareaintegralmeasure", Converter::Missing),
    ("ifcsectionedspine", Converter::Class(Class::IfcSectionedSpine)),
    ("ifcsectionmodulusmeasure", Converter::Missing),
    ("ifcsectionproperties", Converter::NotImplemented),
    ("ifcsectionreinforcementproperties", Converter::NotImplemented),
    ("ifcsectiontypeenum", Converter::Missing),
    ("ifcsensortype", Converter::Class(Class::IfcSensorType)),
    ("ifcsensortypeenum", Converter::Missing),
    ("ifcsequenceenum", Converter::Missing),
    ("ifcservicelife", Converter::Class(Class::IfcServiceLife)),
    ("ifcservicelifefactor", Converter::NotImplemented),
    ("ifcservicelifefactortypeenum", Converter::Missing),
    ("ifcservicelifetypeenum", Converter::Missing),
    ("ifcshapeaspect", Converter::NotImplemented),
    ("ifcshapemodel", Converter::Class(Class::IfcShapeModel)),
    ("ifcshaperepresentation", Converter::Class(Class::IfcShapeRepresentation)),
    ("ifcshearmodulusmeasure", Converter::Missing),
    ("ifcshell", Converter::Missing),
    ("ifcshellbasedsurfacemodel", Converter::Class(Class::IfcShellBasedSurfaceModel)),
    ("ifcsimpleproperty", Converter::Class(Class::IfcSimpleProperty)),
    ("ifcsimplevalue", Converter::Missing),
    ("ifcsiprefix", Converter::Missing),
    ("ifcsite", Converter::Class(Class::IfcSite)),
    ("ifcsiunit", Converter::Class(Class::IfcSIUnit)),
    ("ifcsiunitname", Converter::Missing),
    ("ifcsizeselect", Converter::Missing),
    ("ifcslab", Converter::Class(Class::IfcSlab)),
    ("ifcslabtype", Converter::Class(Class::IfcSlabType)),
    ("ifcslabtypeenum", Converter::Missing),
    ("ifcslippageconnectioncondition", Converter::NotImplemented),
    ("ifcsolidanglemeasure", Converter::Missing),
    ("ifcsolidmodel", Converter::Class(Class::IfcSolidModel)),
    ("ifcsoundpowermeasure", Converter::Missing),
    ("ifcsoundpressuremeasure", Converter::Missing),
    ("ifcsoundproperties", Converter::NotImplemented),
    ("ifcsoundscaleenum", Converter::Missing),
    ("ifcsoundvalue", Converter::NotImplemented),
    ("ifcspace", Converter::Class(Class::IfcSpace)),
    ("ifcspaceheatertype", Converter::Class(Class::IfcSpaceHeaterType)),
    ("ifcspaceheatertypeenum", Converter::Missing),
    ("ifcspaceprogram", Converter::Class(Class::IfcSpaceProgram)),
    ("ifcspacethermalloadproperties", Converter::NotImplemented),
    ("ifcspacetype", Converter::Class(Class::IfcSpaceType)),
    ("ifcspacetypeenum", Converter::Missing),
    ("ifcspatialstructureelement", Converter::Class(Class::IfcSpatialStructureElement)),
    ("ifcspatialstructureelementtype", Converter::Class(Class::IfcSpatialStructureElementType)),
    ("ifcspecificheatcapacitymeasure", Converter::Missing),
    ("ifcspecularexponent", Converter::Missing),
    ("ifcspecularhighlightselect", Converter::Missing),
    ("ifcspecularroughness", Converter::Missing),
    ("ifcsphere", Converter::Class(Class::IfcSphere)),
    ("ifcstackterminaltype", Converter::Class(Class::IfcStackTerminalType)),
    ("ifcstackterminaltypeenum", Converter::Missing),
    ("ifcstair", Converter::Class(Class::IfcStair)),
    ("ifcstairflight", Converter::Class(Class::IfcStairFlight)),
    ("ifcstairflighttype", Converter::Class(Class::IfcStairFlightType)),
    ("ifcstairflighttypeenum", Converter::Missing),
    ("ifcstairtypeenum", Converter::Missing),
    ("ifcstateenum", Converter::Missing),
    ("ifcstructuralaction", Converter::Class(Class::IfcStructuralAction)),
    ("ifcstructuralactivity", Converter::Class(Class::IfcStructuralActivity)),
    ("ifcstructuralactivityassignmentselect", Converter::Missing),
    ("ifcstructuralanalysismodel", Converter::Class(Class::IfcStructuralAnalysisModel)),
    ("ifcstructuralconnection", Converter::Class(Class::IfcStructuralConnection)),
    ("ifcstructuralconnectioncondition", Converter::NotImplemented),
    ("ifcstructuralcurveconnection", Converter::Class(Class::IfcStructuralCurveConnection)),
    ("ifcstructuralcurvemember", Converter::Class(Class::IfcStructuralCurveMember)),
    ("ifcstructuralcurvemembervarying", Converter::Class(Class::IfcStructuralCurveMemberVarying)),
    ("ifcstructuralcurvetypeenum", Converter::Missing),
    ("ifcstructuralitem", Converter::Class(Class::IfcStructuralItem)),
    ("ifcstructurallinearaction", Converter::Class(Class::IfcStructuralLinearAction)),
    ("ifcstructurallinearactionvarying", Converter::Class(Class::IfcStructuralLinearActionVarying)),
    ("ifcstructuralload", Converter::NotImplemented),
    ("ifcstructuralloadgroup", Converter::Class(Class::IfcStructuralLoadGroup)),
    ("ifcstructuralloadlinearforce", Converter::NotImplemented),
    ("ifcstructuralloadplanarforce", Converter::NotImplemented),
    ("ifcstructuralloadsingledisplacement", Converter::NotImplemented),
    ("ifcstructuralloadsingledisplacementdistortion", Converter::NotImplemented),
    ("ifcstructuralloadsingleforce", Converter::NotImplemented),
    ("ifcstructuralloadsingleforcewarping", Converter::NotImplemented),
    ("ifcstructuralloadstatic", Converter::NotImplemented),
    ("ifcstructuralloadtemperature", Converter::NotImplemented),
    ("ifcstructuralmember", Converter::Class(Class::IfcStructuralMember)),
    ("ifcstructuralplanaraction", Converter::Class(Class::IfcStructuralPlanarAction)),
    ("ifcstructuralplanaractionvarying", Converter::Class(Class::IfcStructuralPlanarActionVarying)),
    ("ifcstructuralpointaction", Converter::Class(Class::IfcStructuralPointAction)),
    ("ifcstructuralpointconnection", Converter::Class(Class::IfcStructuralPointConnection)),
    ("ifcstructuralpointreaction", Converter::Class(Class::IfcStructuralPointReaction)),
    ("ifcstructuralprofileproperties", Converter::NotImplemented),
    ("ifcstructuralreaction", Converter::Class(Class::IfcStructuralReaction)),
    ("ifcstructuralresultgroup", Converter::Class(Class::IfcStructuralResultGroup)),
    ("ifcstructuralsteelprofileproperties", Converter::NotImplemented),
    ("ifcstructuralsurfaceconnection", Converter::Class(Class::IfcStructuralSurfaceConnection)),
    ("ifcstructuralsurfacemember", Converter::Class(Class::IfcStructuralSurfaceMember)),
    ("ifcstructuralsurfacemembervarying", Converter::Class(Class::IfcStructuralSurfaceMemberVarying)),
    ("ifcstructuralsurfacetypeenum", Converter::Missing),
    ("ifcstructureddimensioncallout", Converter::Class(Class::IfcStructuredDimensionCallout)),
    ("ifcstyleditem", Converter::Class(Class::IfcStyledItem)),
    ("ifcstyledrepresentation", Converter::Class(Class::IfcStyledRepresentation)),
    ("ifcstylemodel", Converter::Class(Class::IfcStyleModel)),
    ("ifcsubcontractresource", Converter::Class(Class::IfcSubContractResource)),
    ("ifcsubedge", Converter::Class(Class::IfcSubedge)),
    ("ifcsurface", Converter::Class(Class::IfcSurface)),
    ("ifcsurfacecurvesweptareasolid", Converter::Class(Class::IfcSurfaceCurveSweptAreaSolid)),
    ("ifcsurfaceoflinearextrusion", Converter::Class(Class::IfcSurfaceOfLinearExtrusion)),
    ("ifcsurfaceofrevolution", Converter::Class(Class::IfcSurfaceOfRevolution)),
    ("ifcsurfaceorfacesurface", Converter::Missing),
    ("ifcsurfaceside", Converter::Missing),
    ("ifcsurfacestyle", Converter::Class(Class::IfcSurfaceStyle)),
    ("ifcsurfacestyleelementselect", Converter::Missing),
    ("ifcsurfacestylelighting", Converter::NotImplemented),
    ("ifcsurfacestylerefraction", Converter::NotImplemented),
    ("ifcsurfacestylerendering", Converter::Class(Class::IfcSurfaceStyleRendering)),
    ("ifcsurfacestyleshading", Converter::Class(Class::IfcSurfaceStyleShading)),
    ("ifcsurfacestylewithtextures", Converter::Class(Class::IfcSurfaceStyleWithTextures)),
    ("ifcsurfacetexture", Converter::NotImplemented),
    ("ifcsurfacetextureenum", Converter::Missing),
    ("ifcsweptareasolid", Converter::Class(Class::IfcSweptAreaSolid)),
    ("ifcsweptdisksolid", Converter::Class(Class::IfcSweptDiskSolid)),
    ("ifcsweptsurface", Converter::Class(Class::IfcSweptSurface)),
    ("ifcswitchingdevicetype", Converter::Class(Class::IfcSwitchingDeviceType)),
    ("ifcswitchingdevicetypeenum", Converter::Missing),
    ("ifcsymbolstyle", Converter::NotImplemented),
    ("ifcsymbolstyleselect", Converter::Missing),
    ("ifcsystem", Converter::Class(Class::IfcSystem)),
    ("ifcsystemfurnitureelementtype", Converter::Class(Class::IfcSystemFurnitureElementType)),
    ("ifctable", Converter::NotImplemented),
    ("ifctablerow", Converter::NotImplemented),
    ("ifctanktype", Converter::Class(Class::IfcTankType)),
    ("ifctanktypeenum", Converter::Missing),
    ("ifctask", Converter::Class(Class::IfcTask)),
    ("ifctelecomaddress", Converter::NotImplemented),
    ("ifctemperaturegradientmeasure", Converter::Missing),
    ("ifctendon", Converter::Class(Class::IfcTendon)),
    ("ifctendonanchor", Converter::Class(Class::IfcTendonAnchor)),
    ("ifctendontypeenum", Converter::Missing),
    ("ifcterminatorsymbol", Converter::Class(Class::IfcTerminatorSymbol)),
    ("ifctext", Converter::Missing),
    ("ifctextalignment", Converter::Missing),
    ("ifctextdecoration", Converter::Missing),
    ("ifctextfontname", Converter::Missing),
    ("ifctextfontselect", Converter::Missing),
    ("ifctextliteral", Converter::Class(Class::IfcTextLiteral)),
    ("ifctextliteralwithextent", Converter::Class(Class::IfcTextLiteralWithExtent)),
    ("ifctextpath", Converter::Missing),
    ("ifctextstyle", Converter::NotImplemented),
    ("ifctextstylefontmodel", Converter::NotImplemented),
    ("ifctextstylefordefinedfont", Converter::NotImplemented),
    ("ifctextstyleselect", Converter::Missing),
    ("ifctextstyletextmodel", Converter::NotImplemented),
    ("ifctextstylewithboxcharacteristics", Converter::NotImplemented),
    ("ifctexttransformation", Converter::Missing),
    ("ifctexturecoordinate", Converter::NotImplemented),
    ("ifctexturecoordinategenerator", Converter::NotImplemented),
    ("ifctexturemap", Converter::NotImplemented),
    ("ifctexturevertex", Converter::NotImplemented),
    ("ifcthermaladmittancemeasure", Converter::Missing),
    ("ifcthermalconductivitymeasure", Converter::Missing),
    ("ifcthermalexpansioncoefficientmeasure", Converter::Missing),
    ("ifcthermalloadsourceenum", Converter::Missing),
    ("ifcthermalloadtypeenum", Converter::Missing),
    ("ifcthermalmaterialproperties", Converter::NotImplemented),
    ("ifcthermalresistancemeasure", Converter::Missing),
    ("ifcthermaltransmittancemeasure", Converter::Missing),
    ("ifcthermodynamictemperaturemeasure", Converter::Missing),
    ("ifctimemeasure", Converter::Missing),
    ("ifctimeseries", Converter::NotImplemented),
    ("ifctimeseriesdatatypeenum", Converter::Missing),
    ("ifctimeseriesreferencerelationship", Converter::NotImplemented),
    ("ifctimeseriesschedule", Converter::Class(Class::IfcTimeSeriesSchedule)),
    ("ifctimeseriesscheduletypeenum", Converter::Missing),
    ("ifctimeseriesvalue", Converter::NotImplemented),
    ("ifctimestamp", Converter::Missing),
    ("ifctopologicalrepresentationitem", Converter::Class(Class::IfcTopologicalRepresentationItem)),
    ("ifctopologyrepresentation", Converter::Class(Class::IfcTopologyRepresentation)),
    ("ifctorquemeasure", Converter::Missing),
    ("ifctransformertype", Converter::Class(Class::IfcTransformerType)),
    ("ifctransformertypeenum", Converter::Missing),
    ("ifctransitioncode", Converter::Missing),
    ("ifctransportelement", Converter::Class(Class::IfcTransportElement)),
    ("ifctransportelementtype", Converter::Class(Class::IfcTransportElementType)),
    ("ifctransportelementtypeenum", Converter::Missing),
    ("ifctrapeziumprofiledef", Converter::Class(Class::IfcTrapeziumProfileDef)),
    ("ifctrimmedcurve", Converter::Class(Class::IfcTrimmedCurve)),
    ("ifctrimmingpreference", Converter::Missing),
    ("ifctrimmingselect", Converter::Missing),
    ("ifctshapeprofiledef", Converter::Class(Class::IfcTShapeProfileDef)),
    ("ifctubebundletype", Converter::Class(Class::IfcTubeBundleType)),
    ("ifctubebundletypeenum", Converter::Missing),
    ("ifctwodirectionrepeatfactor", Converter::Class(Class::IfcTwoDirectionRepeatFactor)),
    ("ifctypeobject", Converter::Class(Class::IfcTypeObject)),
    ("ifctypeproduct", Converter::Class(Class::IfcTypeProduct)),
    ("ifcunit", Converter::Missing),
    ("ifcunitaryequipmenttype", Converter::Class(Class::IfcUnitaryEquipmentType)),
    ("ifcunitaryequipmenttypeenum", Converter::Missing),
    ("ifcunitassignment", Converter::Class(Class::IfcUnitAssignment)),
    ("ifcunitenum", Converter::Missing),
    ("ifcushapeprofiledef", Converter::Class(Class::IfcUShapeProfileDef)),
    ("ifcvalue", Converter::Missing),
    ("ifcvalvetype", Converter::Class(Class::IfcValveType)),
    ("ifcvalvetypeenum", Converter::Missing),
    ("ifcvaporpermeabilitymeasure", Converter::Missing),
    ("ifcvector", Converter::Class(Class::IfcVector)),
    ("ifcvectorordirection", Converter::Missing),
    ("ifcvertex", Converter::Class(Class::IfcVertex)),
    ("ifcvertexbasedtexturemap", Converter::NotImplemented),
    ("ifcvertexloop", Converter::Class(Class::IfcVertexLoop)),
    ("ifcvertexpoint", Converter::Class(Class::IfcVertexPoint)),
    ("ifcvibrationisolatortype", Converter::Class(Class::IfcVibrationIsolatorType)),
    ("ifcvibrationisolatortypeenum", Converter::Missing),
    ("ifcvirtualelement", Converter::Class(Class::IfcVirtualElement)),
    ("ifcvirtualgridintersection", Converter::NotImplemented),
    ("ifcvolumemeasure", Converter::Missing),
    ("ifcvolumetricflowratemeasure", Converter::Missing),
    ("ifcwall", Converter::Class(Class::IfcWall)),
    ("ifcwallstandardcase", Converter::Class(Class::IfcWallStandardCase)),
    ("ifcwalltype", Converter::Class(Class::IfcWallType)),
    ("ifcwalltypeenum", Converter::Missing),
    ("ifcwarpingconstantmeasure", Converter::Missing),
    ("ifcwarpingmomentmeasure", Converter::Missing),
    ("ifcwasteterminaltype", Converter::Class(Class::IfcWasteTerminalType)),
    ("ifcwasteterminaltypeenum", Converter::Missing),
    ("ifcwaterproperties", Converter::NotImplemented),
    ("ifcwindow", Converter::Class(Class::IfcWindow)),
    ("ifcwindowliningproperties", Converter::NotImplemented),
    ("ifcwindowpaneloperationenum", Converter::Missing),
    ("ifcwindowpanelpositionenum", Converter::Missing),
    ("ifcwindowpanelproperties", Converter::NotImplemented),
    ("ifcwindowstyle", Converter::Class(Class::IfcWindowStyle)),
    ("ifcwindowstyleconstructionenum", Converter::Missing),
    ("ifcwindowstyleoperationenum", Converter::Missing),
    ("ifcworkcontrol", Converter::Class(Class::IfcWorkControl)),
    ("ifcworkcontroltypeenum", Converter::Missing),
    ("ifcworkplan", Converter::Class(Class::IfcWorkPlan)),
    ("ifcworkschedule", Converter::Class(Class::IfcWorkSchedule)),
    ("ifcyearnumber", Converter::Missing),
    ("ifczone", Converter::Class(Class::IfcZone)),
    ("ifczshapeprofiledef", Converter::Class(Class::IfcZShapeProfileDef)),
];

/// `GetConverterProc` (and whether the name is in the schema at all).
pub(crate) fn converter(name: &[u8]) -> Option<Converter> {
    let index = SCHEMA.binary_search_by(|(entry, _)| entry.as_bytes().cmp(name)).ok()?;
    Some(SCHEMA[index].1)
}
